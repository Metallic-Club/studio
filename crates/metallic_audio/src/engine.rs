//! [`AudioEngine`]: the game-thread interface (one per player process or test).

use crate::asset::{DecoderStream, SoundAsset, StreamSource};
use crate::backend::AudioBackend;
use crate::effects::{Effect, EffectState};
use crate::error::AudioError;
use crate::mixer::{
    Command, EffectSlot, Group, MixRenderer, RendererParts, STATE_PAUSED, STATE_PLAYING,
    SharedStats, SoundShared, Source, StreamVoice, Trash, Voice,
};
use crate::types::Emitter;
use crate::types::{
    AssetId, EffectId, EngineConfig, GroupId, Listener, MixStats, SoundEvent, SoundId,
    SoundProperties, SpatialSettings,
};
use glam::Vec3;
use std::collections::HashMap;
use std::mem::Discriminant;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender, channel};

/// Answers "how blocked is the path from the listener to this sound?" with 0 (clear) to 1
/// (fully blocked). Arguments: listener position, emitter position. The runtime implements it
/// with a physics raycast (for example: 1 if any non-transparent part is hit before the sound).
pub type OcclusionHook = Box<dyn FnMut(Vec3, Vec3) -> f32 + Send>;

enum Output {
    Headless {
        renderer: Box<MixRenderer>,
        clocked: bool,
        carry: f64,
        scratch: Vec<f32>,
    },
    Backend(Box<dyn AudioBackend>),
}

enum AssetEntry {
    Decoded(Arc<SoundAsset>),
    Encoded {
        bytes: Arc<[u8]>,
        extension: Option<String>,
    },
}

struct SoundRecord {
    props: SoundProperties,
    shared: Arc<SoundShared>,
    /// Sequence number of the last transport command (play/pause/resume/stop/seek).
    seq: u64,
    playing_intent: bool,
    paused_intent: bool,
    /// Sequence number of the last command that set the position, and that position.
    position_seq: u64,
    position_intent: f64,
    /// `TimePosition` last set by script; `Play()` starts here (Roblox behaviour).
    script_position: f64,
    seen_loops: u32,
    seen_ended: u32,
    occlusion: f32,
}

struct SoundSlot {
    generation: u32,
    record: Option<SoundRecord>,
}

struct GroupRecord {
    parent: Option<GroupId>,
    volume: f32,
    effects: usize,
}

struct EffectRecord {
    group: GroupId,
    kind: Discriminant<Effect>,
}

/// The audio engine: assets, sounds, groups, effects, listener and output.
///
/// All methods are called from the game thread. Changes reach the audio thread through a
/// command queue and take effect at the next mix block (1.33 ms at 48 kHz); reads such as
/// [`is_playing`](Self::is_playing) reflect commands immediately, then the audio thread's truth.
///
/// ```
/// use metallic_audio::{AudioEngine, EngineConfig, SoundAsset, SoundProperties};
/// let mut engine = AudioEngine::headless(EngineConfig::default())?;
/// let tone: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin()).collect();
/// let asset = engine.insert_asset(SoundAsset::from_mono(tone, 48_000)?);
/// let sound = engine.create_sound(asset, SoundProperties { playing: true, ..Default::default() })?;
/// let mut out = vec![0.0; 2 * 480];
/// engine.render(&mut out)?;
/// assert!(engine.is_playing(sound)?);
/// # Ok::<(), metallic_audio::AudioError>(())
/// ```
pub struct AudioEngine {
    config: EngineConfig,
    sample_rate: u32,
    commands: Sender<Command>,
    trash: Receiver<Trash>,
    output: Output,
    stats: Arc<SharedStats>,
    assets: Vec<Option<AssetEntry>>,
    sounds: Vec<SoundSlot>,
    free_sounds: Vec<u32>,
    groups: Vec<Option<GroupRecord>>,
    effects: HashMap<u32, EffectRecord>,
    next_effect: u32,
    next_seq: u64,
    listener: Listener,
    spatial: SpatialSettings,
    occlusion_hook: Option<OcclusionHook>,
    events: Vec<SoundEvent>,
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEngine")
            .field("backend", &self.backend_name())
            .field("sample_rate", &self.sample_rate)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl AudioEngine {
    /// An engine with no device: nothing is heard, and time only advances when
    /// [`render`](Self::render) or [`advance`](Self::advance) is called. Deterministic: the same
    /// calls always produce bit-identical samples. Used by tests and offline rendering.
    pub fn headless(config: EngineConfig) -> Result<Self, AudioError> {
        Self::build(config, config.sample_rate, None, false)
    }

    /// A headless engine whose time advances with [`update`](Self::update)'s `dt`, so
    /// `TimePosition`, `Ended` and `DidLoop` behave as if audio were playing. For servers and
    /// for players on machines without an output device.
    pub fn headless_clocked(config: EngineConfig) -> Result<Self, AudioError> {
        Self::build(config, config.sample_rate, None, true)
    }

    /// An engine that plays through `backend`.
    pub fn with_backend(
        backend: Box<dyn AudioBackend>,
        config: EngineConfig,
    ) -> Result<Self, AudioError> {
        let rate = backend.sample_rate();
        Self::build(config, rate, Some(backend), false)
    }

    /// An engine on the default output device.
    #[cfg(feature = "device")]
    pub fn open_device(config: EngineConfig) -> Result<Self, AudioError> {
        let backend = crate::device::DeviceBackend::open()?;
        Self::with_backend(Box::new(backend), config)
    }

    /// The default device if there is one, otherwise a clocked headless engine (the game keeps
    /// running silently; see [`headless_clocked`](Self::headless_clocked)).
    #[cfg(feature = "device")]
    pub fn open_device_or_headless(config: EngineConfig) -> Result<Self, AudioError> {
        match Self::open_device(config) {
            Ok(engine) => Ok(engine),
            Err(_) => Self::headless_clocked(config),
        }
    }

    fn build(
        config: EngineConfig,
        sample_rate: u32,
        backend: Option<Box<dyn AudioBackend>>,
        clocked: bool,
    ) -> Result<Self, AudioError> {
        if !(8_000..=384_000).contains(&sample_rate) {
            return Err(AudioError::InvalidParameter(format!(
                "sample rate {sample_rate} Hz"
            )));
        }
        if config.max_voices == 0 || config.max_sounds == 0 {
            return Err(AudioError::InvalidParameter(
                "max_voices and max_sounds must be at least 1".into(),
            ));
        }
        if config.max_sounds > u32::MAX as usize {
            return Err(AudioError::InvalidParameter(
                "max_sounds is too large".into(),
            ));
        }
        if config.max_groups < GroupId::BUILT_IN.len() {
            return Err(AudioError::InvalidParameter(
                "max_groups must cover the 5 built-in groups".into(),
            ));
        }
        let (commands, command_rx) = channel();
        let (trash_tx, trash) = channel();
        let stats = Arc::new(SharedStats::default());
        let renderer = MixRenderer::new(RendererParts {
            sample_rate,
            max_voices: config.max_voices,
            max_sounds: config.max_sounds,
            max_groups: config.max_groups,
            max_effects: config.max_effects_per_group,
            commands: command_rx,
            trash: trash_tx,
            stats: Arc::clone(&stats),
        });
        let output = match backend {
            Some(mut backend) => {
                backend.start(renderer)?;
                Output::Backend(backend)
            }
            None => Output::Headless {
                renderer: Box::new(renderer),
                clocked,
                carry: 0.0,
                scratch: Vec::new(),
            },
        };
        let mut groups: Vec<Option<GroupRecord>> = (0..config.max_groups).map(|_| None).collect();
        for (i, id) in GroupId::BUILT_IN.iter().enumerate() {
            let parent = if *id == GroupId::MASTER {
                None
            } else {
                Some(GroupId::MASTER)
            };
            groups[i] = Some(GroupRecord {
                parent,
                volume: 1.0,
                effects: 0,
            });
        }
        Ok(Self {
            config,
            sample_rate,
            commands,
            trash,
            output,
            stats,
            assets: Vec::new(),
            sounds: Vec::new(),
            free_sounds: Vec::new(),
            groups,
            effects: HashMap::new(),
            next_effect: 1,
            next_seq: 1,
            listener: Listener::default(),
            spatial: SpatialSettings::default(),
            occlusion_hook: None,
            events: Vec::new(),
        })
    }

    // -----------------------------------------------------------------------------------------
    // Engine

    /// Output sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The configuration the engine was built with.
    pub fn config(&self) -> EngineConfig {
        self.config
    }

    /// `"headless"` or the backend's device name.
    pub fn backend_name(&self) -> &str {
        match &self.output {
            Output::Headless { .. } => "headless",
            Output::Backend(b) => b.name(),
        }
    }

    /// True when no device is attached.
    pub fn is_headless(&self) -> bool {
        matches!(self.output, Output::Headless { .. })
    }

    /// The backend's last asynchronous error (device unplugged and similar).
    pub fn backend_error(&self) -> Option<String> {
        match &self.output {
            Output::Headless { .. } => None,
            Output::Backend(b) => b.last_error(),
        }
    }

    /// Voices in use and frames rendered.
    pub fn stats(&self) -> MixStats {
        MixStats {
            real_voices: self.stats.real_voices.load(Ordering::Relaxed),
            virtual_voices: self.stats.virtual_voices.load(Ordering::Relaxed),
            frames_rendered: self.stats.frames.load(Ordering::Acquire),
        }
    }

    fn send(&self, command: Command) {
        // The receiver lives as long as the backend or the headless renderer, both owned by
        // `self`, so this only fails if a backend dropped its renderer; nothing to do then.
        let _ = self.commands.send(command);
    }

    fn seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    /// Per-frame housekeeping on the game thread: advance a clocked headless engine by `dt`
    /// seconds, ask the occlusion hook about every playing positional sound, collect
    /// `Ended`/`DidLoop` events and free objects the audio thread released.
    pub fn update(&mut self, dt: f32) {
        let mut hook = self.occlusion_hook.take();
        match hook.as_mut() {
            Some(h) => self.update_inner(dt, Some(&mut **h)),
            None => self.update_inner(dt, None),
        }
        self.occlusion_hook = hook;
    }

    /// [`update`](Self::update) with an occlusion query that may borrow game state for this
    /// call only (for example `|from, to| physics.raycast(...)`). Overrides the stored hook for
    /// this frame.
    pub fn update_with_occlusion(&mut self, dt: f32, query: &mut dyn FnMut(Vec3, Vec3) -> f32) {
        self.update_inner(dt, Some(query));
    }

    fn update_inner(&mut self, dt: f32, query: Option<&mut dyn FnMut(Vec3, Vec3) -> f32>) {
        if let Output::Headless {
            renderer,
            clocked: true,
            carry,
            scratch,
        } = &mut self.output
        {
            let exact = (dt.max(0.0) as f64) * self.sample_rate as f64 + *carry;
            let frames = exact.floor();
            *carry = exact - frames;
            let frames = frames as usize;
            if scratch.len() < frames * 2 {
                scratch.resize(frames * 2, 0.0);
            }
            renderer.render(&mut scratch[..frames * 2]);
        }
        if let Some(query) = query {
            self.run_occlusion(query);
        }
        while self.trash.try_recv().is_ok() {}
        self.collect_events();
    }

    fn run_occlusion(&mut self, hook: &mut dyn FnMut(Vec3, Vec3) -> f32) {
        let listener = self.listener.position;
        let mut changes = Vec::new();
        for (index, slot) in self.sounds.iter_mut().enumerate() {
            let Some(rec) = slot.record.as_mut() else {
                continue;
            };
            let Some(emitter) = rec.props.emitter else {
                continue;
            };
            if !record_is_playing(rec) {
                continue;
            }
            let raw = hook(listener, emitter.position);
            let occlusion = if raw.is_finite() {
                raw.clamp(0.0, 1.0)
            } else {
                0.0
            };
            if (occlusion - rec.occlusion).abs() > 1e-3 {
                rec.occlusion = occlusion;
                changes.push(Command::SetOcclusion { index, occlusion });
            }
        }
        for command in changes {
            self.send(command);
        }
    }

    fn collect_events(&mut self) {
        for (index, slot) in self.sounds.iter_mut().enumerate() {
            let Some(rec) = slot.record.as_mut() else {
                continue;
            };
            let id = SoundId {
                index: index as u32,
                generation: slot.generation,
            };
            let loops = rec.shared.loops.load(Ordering::Acquire);
            if loops != rec.seen_loops {
                rec.seen_loops = loops;
                self.events.push(SoundEvent::DidLoop {
                    sound: id,
                    count: loops,
                });
            }
            let ended = rec.shared.ended.load(Ordering::Acquire);
            if ended != rec.seen_ended {
                rec.seen_ended = ended;
                self.events.push(SoundEvent::Ended(id));
            }
        }
    }

    /// Events gathered by [`update`](Self::update) since the last call, oldest first.
    pub fn drain_events(&mut self) -> Vec<SoundEvent> {
        std::mem::take(&mut self.events)
    }

    /// Headless only: render `out.len() / 2` stereo frames (interleaved) and advance time.
    pub fn render(&mut self, out: &mut [f32]) -> Result<(), AudioError> {
        match &mut self.output {
            Output::Headless { renderer, .. } => {
                renderer.render(out);
                Ok(())
            }
            Output::Backend(_) => Err(AudioError::NotHeadless),
        }
    }

    /// Headless only: render `seconds` of audio into a new interleaved stereo buffer.
    pub fn render_seconds(&mut self, seconds: f64) -> Result<Vec<f32>, AudioError> {
        let frames = (seconds.max(0.0) * self.sample_rate as f64).round() as usize;
        let mut out = vec![0.0; frames * 2];
        self.render(&mut out)?;
        Ok(out)
    }

    /// Headless only: advance time by `seconds`, discarding the output.
    pub fn advance(&mut self, seconds: f64) -> Result<(), AudioError> {
        self.render_seconds(seconds).map(|_| ())
    }

    // -----------------------------------------------------------------------------------------
    // Listener and world settings

    /// Move the listener (call every frame with the camera).
    pub fn set_listener(&mut self, listener: Listener) {
        self.listener = listener;
        self.send(Command::SetListener(listener));
    }

    /// The current listener.
    pub fn listener(&self) -> Listener {
        self.listener
    }

    /// Change the world-wide spatial settings (`SoundService` properties).
    pub fn set_spatial_settings(&mut self, settings: SpatialSettings) {
        self.spatial = settings;
        self.send(Command::SetSpatial(settings));
    }

    /// The current spatial settings.
    pub fn spatial_settings(&self) -> SpatialSettings {
        self.spatial
    }

    /// Install the occlusion query, called from [`update`](Self::update) for each playing
    /// positional sound.
    pub fn set_occlusion_hook(&mut self, hook: impl FnMut(Vec3, Vec3) -> f32 + Send + 'static) {
        self.occlusion_hook = Some(Box::new(hook));
    }

    /// Remove the occlusion query and clear all occlusion.
    pub fn clear_occlusion_hook(&mut self) {
        self.occlusion_hook = None;
        let mut changes = Vec::new();
        for (index, slot) in self.sounds.iter_mut().enumerate() {
            if let Some(rec) = slot.record.as_mut()
                && rec.occlusion != 0.0
            {
                rec.occlusion = 0.0;
                changes.push(Command::SetOcclusion {
                    index,
                    occlusion: 0.0,
                });
            }
        }
        for command in changes {
            self.send(command);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Assets

    /// Register decoded PCM.
    pub fn insert_asset(&mut self, asset: SoundAsset) -> AssetId {
        self.push_asset(AssetEntry::Decoded(Arc::new(asset)))
    }

    /// Decode a whole file from memory (WAV, OGG Vorbis, FLAC, MP3) and register it. Best for
    /// short sounds; use [`load_streaming`](Self::load_streaming) for music.
    pub fn load_asset(
        &mut self,
        bytes: &[u8],
        extension: Option<&str>,
    ) -> Result<AssetId, AudioError> {
        let asset = SoundAsset::decode(bytes, extension)?;
        Ok(self.insert_asset(asset))
    }

    /// Register an encoded file that is decoded while it plays. The file is probed now, so a
    /// broken or unsupported file fails here rather than at play time.
    pub fn load_streaming(
        &mut self,
        bytes: impl Into<Arc<[u8]>>,
        extension: Option<&str>,
    ) -> Result<AssetId, AudioError> {
        let bytes = bytes.into();
        DecoderStream::from_bytes(Arc::clone(&bytes), extension)?;
        Ok(self.push_asset(AssetEntry::Encoded {
            bytes,
            extension: extension.map(str::to_owned),
        }))
    }

    fn push_asset(&mut self, entry: AssetEntry) -> AssetId {
        if let Some(i) = self.assets.iter().position(Option::is_none) {
            self.assets[i] = Some(entry);
            AssetId(i as u32)
        } else {
            self.assets.push(Some(entry));
            AssetId(self.assets.len() as u32 - 1)
        }
    }

    /// Forget an asset. Sounds already created from it keep playing.
    pub fn remove_asset(&mut self, asset: AssetId) -> Result<(), AudioError> {
        match self.assets.get_mut(asset.0 as usize) {
            Some(slot @ Some(_)) => {
                *slot = None;
                Ok(())
            }
            _ => Err(AudioError::UnknownAsset(asset)),
        }
    }

    /// Length in seconds of a loaded asset, if known without decoding.
    pub fn asset_length(&self, asset: AssetId) -> Result<Option<f64>, AudioError> {
        match self.assets.get(asset.0 as usize).and_then(Option::as_ref) {
            Some(AssetEntry::Decoded(a)) => Ok(Some(a.duration_secs())),
            Some(AssetEntry::Encoded { .. }) => Ok(None),
            None => Err(AudioError::UnknownAsset(asset)),
        }
    }

    // -----------------------------------------------------------------------------------------
    // Sounds

    /// Create a sound (a `Sound` instance) playing `asset` with `props`. If `props.playing`
    /// is set it starts at `props.time_position`.
    pub fn create_sound(
        &mut self,
        asset: AssetId,
        props: SoundProperties,
    ) -> Result<SoundId, AudioError> {
        let source = match self.assets.get(asset.0 as usize).and_then(Option::as_ref) {
            Some(AssetEntry::Decoded(a)) => Source::Asset(Arc::clone(a)),
            Some(AssetEntry::Encoded { bytes, extension }) => {
                let stream = DecoderStream::from_bytes(Arc::clone(bytes), extension.as_deref())?;
                Source::Stream(Box::new(StreamVoice::new(Box::new(stream))))
            }
            None => return Err(AudioError::UnknownAsset(asset)),
        };
        self.create_from_source(source, props)
    }

    /// Create a sound fed by a custom [`StreamSource`] (procedural audio; later voice chat).
    pub fn create_stream_sound(
        &mut self,
        source: Box<dyn StreamSource>,
        props: SoundProperties,
    ) -> Result<SoundId, AudioError> {
        if source.sample_rate() == 0 || !(1..=2).contains(&source.channels()) {
            return Err(AudioError::InvalidParameter(
                "stream needs 1 or 2 channels and a sample rate".into(),
            ));
        }
        self.create_from_source(Source::Stream(Box::new(StreamVoice::new(source))), props)
    }

    fn create_from_source(
        &mut self,
        source: Source,
        props: SoundProperties,
    ) -> Result<SoundId, AudioError> {
        let props = sanitize(props);
        let group = self.check_group(props.sound_group)?;
        let index = match self.free_sounds.pop() {
            Some(i) => i as usize,
            None if self.sounds.len() < self.config.max_sounds => {
                self.sounds.push(SoundSlot {
                    generation: 0,
                    record: None,
                });
                self.sounds.len() - 1
            }
            None => return Err(AudioError::Limit("max_sounds")),
        };
        let length = match &source {
            Source::Asset(a) => Some(a.duration_secs()),
            Source::Stream(_) => None,
        };
        let shared = Arc::new(SoundShared::new(length));
        let mut voice = Voice::new(source, Arc::clone(&shared), group.0 as usize);
        voice.volume = props.volume;
        voice.speed = props.playback_speed;
        voice.looped = props.looped;
        voice.priority = props.priority;
        voice.emitter = props.emitter;
        voice.roll_off = props.roll_off();
        self.send(Command::AddSound {
            index,
            voice: Box::new(voice),
        });

        let slot = &mut self.sounds[index];
        slot.generation = slot.generation.wrapping_add(1);
        slot.record = Some(SoundRecord {
            props,
            shared,
            seq: 0,
            playing_intent: false,
            paused_intent: false,
            position_seq: 0,
            position_intent: 0.0,
            script_position: props.time_position,
            seen_loops: 0,
            seen_ended: 0,
            occlusion: 0.0,
        });
        let id = SoundId {
            index: index as u32,
            generation: slot.generation,
        };
        if props.playing {
            self.play(id)?;
        } else if props.time_position > 0.0 {
            self.set_time_position(id, props.time_position)?;
        }
        Ok(id)
    }

    /// Destroy a sound. Its handle becomes invalid.
    pub fn remove_sound(&mut self, sound: SoundId) -> Result<(), AudioError> {
        self.record(sound)?;
        let slot = &mut self.sounds[sound.index as usize];
        slot.record = None;
        self.free_sounds.push(sound.index);
        self.send(Command::RemoveSound {
            index: sound.index as usize,
        });
        Ok(())
    }

    /// Number of live sounds.
    pub fn sound_count(&self) -> usize {
        self.sounds.iter().filter(|s| s.record.is_some()).count()
    }

    fn record(&self, sound: SoundId) -> Result<&SoundRecord, AudioError> {
        self.sounds
            .get(sound.index as usize)
            .filter(|s| s.generation == sound.generation)
            .and_then(|s| s.record.as_ref())
            .ok_or(AudioError::UnknownSound(sound))
    }

    fn record_mut(&mut self, sound: SoundId) -> Result<&mut SoundRecord, AudioError> {
        self.sounds
            .get_mut(sound.index as usize)
            .filter(|s| s.generation == sound.generation)
            .and_then(|s| s.record.as_mut())
            .ok_or(AudioError::UnknownSound(sound))
    }

    /// `Sound:Play()`: start from the `TimePosition` last set by script (0 if never set),
    /// restarting if already playing.
    pub fn play(&mut self, sound: SoundId) -> Result<(), AudioError> {
        let seq = self.seq();
        let rec = self.record_mut(sound)?;
        let from = rec.script_position;
        rec.seq = seq;
        rec.position_seq = seq;
        rec.position_intent = from;
        rec.playing_intent = true;
        rec.paused_intent = false;
        rec.props.playing = true;
        self.send(Command::Play {
            index: sound.index as usize,
            seq,
            from_secs: from,
        });
        Ok(())
    }

    /// `Sound:Pause()`: stop where it is; [`resume`](Self::resume) continues.
    pub fn pause(&mut self, sound: SoundId) -> Result<(), AudioError> {
        let seq = self.seq();
        let rec = self.record_mut(sound)?;
        let was_active = record_is_playing(rec) || record_is_paused(rec);
        rec.seq = seq;
        rec.playing_intent = false;
        rec.paused_intent = was_active;
        rec.props.playing = false;
        self.send(Command::Pause {
            index: sound.index as usize,
            seq,
        });
        Ok(())
    }

    /// `Sound:Resume()`: continue from the current position.
    pub fn resume(&mut self, sound: SoundId) -> Result<(), AudioError> {
        let seq = self.seq();
        let rec = self.record_mut(sound)?;
        rec.seq = seq;
        rec.playing_intent = true;
        rec.paused_intent = false;
        rec.props.playing = true;
        self.send(Command::Resume {
            index: sound.index as usize,
            seq,
        });
        Ok(())
    }

    /// `Sound:Stop()`: stop and reset `TimePosition` to 0.
    pub fn stop(&mut self, sound: SoundId) -> Result<(), AudioError> {
        let seq = self.seq();
        let rec = self.record_mut(sound)?;
        rec.seq = seq;
        rec.position_seq = seq;
        rec.position_intent = 0.0;
        rec.script_position = 0.0;
        rec.playing_intent = false;
        rec.paused_intent = false;
        rec.props.playing = false;
        self.send(Command::Stop {
            index: sound.index as usize,
            seq,
        });
        Ok(())
    }

    /// Setting `Sound.Playing`: `true` resumes from the current position, `false` pauses.
    pub fn set_playing(&mut self, sound: SoundId, playing: bool) -> Result<(), AudioError> {
        if playing {
            self.resume(sound)
        } else {
            self.pause(sound)
        }
    }

    /// `Sound.IsPlaying`.
    pub fn is_playing(&self, sound: SoundId) -> Result<bool, AudioError> {
        Ok(record_is_playing(self.record(sound)?))
    }

    /// `Sound.IsPaused`.
    pub fn is_paused(&self, sound: SoundId) -> Result<bool, AudioError> {
        Ok(record_is_paused(self.record(sound)?))
    }

    /// `Sound.TimePosition` in seconds.
    pub fn time_position(&self, sound: SoundId) -> Result<f64, AudioError> {
        let rec = self.record(sound)?;
        if rec.shared.applied_seq.load(Ordering::Acquire) < rec.position_seq {
            Ok(rec.position_intent)
        } else {
            Ok(rec.shared.position_secs())
        }
    }

    /// Setting `Sound.TimePosition`: jump to `seconds` (clamped to the length); also where the
    /// next `Play()` starts.
    pub fn set_time_position(&mut self, sound: SoundId, seconds: f64) -> Result<(), AudioError> {
        if !seconds.is_finite() {
            return Err(AudioError::InvalidParameter(format!(
                "TimePosition {seconds}"
            )));
        }
        let seconds = seconds.max(0.0);
        let seq = self.seq();
        let rec = self.record_mut(sound)?;
        let clamped = rec
            .shared
            .length_secs()
            .map_or(seconds, |len| seconds.min(len));
        rec.seq = seq;
        rec.position_seq = seq;
        rec.position_intent = clamped;
        rec.script_position = clamped;
        rec.props.time_position = clamped;
        self.send(Command::Seek {
            index: sound.index as usize,
            seq,
            secs: clamped,
        });
        Ok(())
    }

    /// `Sound.TimeLength` in seconds (`None` for a stream whose end has not been reached and
    /// whose container does not state its length).
    pub fn time_length(&self, sound: SoundId) -> Result<Option<f64>, AudioError> {
        Ok(self.record(sound)?.shared.length_secs())
    }

    /// The sound's current properties (`playing` and `time_position` as last set by the game).
    pub fn properties(&self, sound: SoundId) -> Result<SoundProperties, AudioError> {
        Ok(self.record(sound)?.props)
    }

    /// `Sound.Volume` (0..=10).
    pub fn set_volume(&mut self, sound: SoundId, volume: f32) -> Result<(), AudioError> {
        let volume = clamp_finite(volume, 0.0, 10.0, "Volume")?;
        self.record_mut(sound)?.props.volume = volume;
        self.send(Command::SetVolume {
            index: sound.index as usize,
            volume,
        });
        Ok(())
    }

    /// `Sound.PlaybackSpeed` (0..=20).
    pub fn set_playback_speed(&mut self, sound: SoundId, speed: f32) -> Result<(), AudioError> {
        let speed = clamp_finite(speed, 0.0, 20.0, "PlaybackSpeed")?;
        self.record_mut(sound)?.props.playback_speed = speed;
        self.send(Command::SetSpeed {
            index: sound.index as usize,
            speed,
        });
        Ok(())
    }

    /// `Sound.Looped`.
    pub fn set_looped(&mut self, sound: SoundId, looped: bool) -> Result<(), AudioError> {
        self.record_mut(sound)?.props.looped = looped;
        self.send(Command::SetLooped {
            index: sound.index as usize,
            looped,
        });
        Ok(())
    }

    /// `Sound.RollOffMode`, `RollOffMinDistance`, `RollOffMaxDistance` together.
    pub fn set_roll_off(
        &mut self,
        sound: SoundId,
        mode: crate::RollOffMode,
        min_distance: f32,
        max_distance: f32,
    ) -> Result<(), AudioError> {
        let min = clamp_finite(min_distance, 0.0, f32::MAX, "RollOffMinDistance")?;
        let max = clamp_finite(max_distance, 0.0, f32::MAX, "RollOffMaxDistance")?.max(min);
        let rec = self.record_mut(sound)?;
        rec.props.roll_off_mode = mode;
        rec.props.roll_off_min_distance = min;
        rec.props.roll_off_max_distance = max;
        let roll_off = rec.props.roll_off();
        self.send(Command::SetRollOff {
            index: sound.index as usize,
            roll_off,
        });
        Ok(())
    }

    /// Move a positional sound or make it non-positional (`None`). Call each frame for sounds on
    /// moving parts.
    pub fn set_emitter(
        &mut self,
        sound: SoundId,
        emitter: Option<Emitter>,
    ) -> Result<(), AudioError> {
        if let Some(e) = emitter
            && !(e.position.is_finite() && e.velocity.is_finite())
        {
            return Err(AudioError::InvalidParameter(
                "emitter position/velocity must be finite".into(),
            ));
        }
        self.record_mut(sound)?.props.emitter = emitter;
        self.send(Command::SetEmitter {
            index: sound.index as usize,
            emitter,
        });
        Ok(())
    }

    /// `Sound.SoundGroup`.
    pub fn set_sound_group(&mut self, sound: SoundId, group: GroupId) -> Result<(), AudioError> {
        self.check_group(group)?;
        self.record_mut(sound)?.props.sound_group = group;
        self.send(Command::SetGroup {
            index: sound.index as usize,
            group: group.0 as usize,
        });
        Ok(())
    }

    /// Voice-limit priority (higher is kept when voices run out).
    pub fn set_priority(&mut self, sound: SoundId, priority: i32) -> Result<(), AudioError> {
        self.record_mut(sound)?.props.priority = priority;
        self.send(Command::SetPriority {
            index: sound.index as usize,
            priority,
        });
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Groups

    fn check_group(&self, group: GroupId) -> Result<GroupId, AudioError> {
        match self.groups.get(group.0 as usize) {
            Some(Some(_)) => Ok(group),
            _ => Err(AudioError::UnknownGroup(group)),
        }
    }

    /// Create a `SoundGroup` under `parent` with volume 1.
    pub fn create_group(&mut self, parent: GroupId) -> Result<GroupId, AudioError> {
        self.check_group(parent)?;
        // A new group always gets a higher index than its parent, so the mixer can process
        // children before parents by walking indices downwards.
        let index = self
            .groups
            .iter()
            .enumerate()
            .skip(parent.0 as usize + 1)
            .find(|(_, g)| g.is_none())
            .map(|(i, _)| i)
            .ok_or(AudioError::Limit("max_groups"))?;
        self.groups[index] = Some(GroupRecord {
            parent: Some(parent),
            volume: 1.0,
            effects: 0,
        });
        let group = Group::new(
            Some(parent.0 as usize),
            1.0,
            self.config.max_effects_per_group,
        );
        self.send(Command::AddGroup {
            index,
            group: Box::new(group),
        });
        Ok(GroupId(index as u32))
    }

    /// Remove a created group. Its sounds and child groups move to its parent; its effects are
    /// dropped. Built-in groups cannot be removed.
    pub fn remove_group(&mut self, group: GroupId) -> Result<(), AudioError> {
        self.check_group(group)?;
        if group.is_built_in() {
            return Err(AudioError::InvalidParameter(
                "built-in groups cannot be removed".into(),
            ));
        }
        let parent = self.groups[group.0 as usize]
            .as_ref()
            .and_then(|g| g.parent)
            .unwrap_or(GroupId::MASTER);
        let children: Vec<usize> = self
            .groups
            .iter()
            .enumerate()
            .filter(|(_, g)| g.as_ref().is_some_and(|g| g.parent == Some(group)))
            .map(|(i, _)| i)
            .collect();
        for child in children {
            if let Some(g) = self.groups[child].as_mut() {
                g.parent = Some(parent);
            }
            self.send(Command::SetGroupParent {
                index: child,
                parent: parent.0 as usize,
            });
        }
        let moved: Vec<SoundId> = self
            .sounds
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let rec = s.record.as_ref()?;
                (rec.props.sound_group == group).then_some(SoundId {
                    index: i as u32,
                    generation: s.generation,
                })
            })
            .collect();
        for sound in moved {
            self.set_sound_group(sound, parent)?;
        }
        self.effects.retain(|_, e| e.group != group);
        self.groups[group.0 as usize] = None;
        self.send(Command::RemoveGroup {
            index: group.0 as usize,
        });
        Ok(())
    }

    /// `SoundGroup.Volume` (0..=10). Built-in groups start at 1.
    pub fn set_group_volume(&mut self, group: GroupId, volume: f32) -> Result<(), AudioError> {
        let volume = clamp_finite(volume, 0.0, 10.0, "SoundGroup.Volume")?;
        self.check_group(group)?;
        if let Some(g) = self.groups[group.0 as usize].as_mut() {
            g.volume = volume;
        }
        self.send(Command::SetGroupVolume {
            index: group.0 as usize,
            volume,
        });
        Ok(())
    }

    /// A group's own volume.
    pub fn group_volume(&self, group: GroupId) -> Result<f32, AudioError> {
        self.check_group(group)?;
        Ok(self.groups[group.0 as usize]
            .as_ref()
            .map_or(0.0, |g| g.volume))
    }

    /// The group's parent (`None` for Master).
    pub fn group_parent(&self, group: GroupId) -> Result<Option<GroupId>, AudioError> {
        self.check_group(group)?;
        Ok(self.groups[group.0 as usize]
            .as_ref()
            .and_then(|g| g.parent))
    }

    /// Product of the volumes from `group` up to Master: what a sound's volume is multiplied
    /// by when it plays in this group (before effects).
    pub fn effective_group_volume(&self, group: GroupId) -> Result<f32, AudioError> {
        self.check_group(group)?;
        let mut gain = 1.0;
        let mut current = Some(group);
        while let Some(g) = current
            .and_then(|g| self.groups.get(g.0 as usize))
            .and_then(Option::as_ref)
        {
            gain *= g.volume;
            current = g.parent;
        }
        Ok(gain)
    }

    // -----------------------------------------------------------------------------------------
    // Effects

    /// Insert an effect on a group. Effects run in descending `priority` (Roblox
    /// `SoundEffect.Priority`), equal priorities in insertion order, before the group volume.
    pub fn add_effect(
        &mut self,
        group: GroupId,
        effect: Effect,
        priority: i32,
    ) -> Result<EffectId, AudioError> {
        self.check_group(group)?;
        let max = self.config.max_effects_per_group;
        let record = self.groups[group.0 as usize]
            .as_mut()
            .ok_or(AudioError::UnknownGroup(group))?;
        if record.effects >= max {
            return Err(AudioError::Limit("max_effects_per_group"));
        }
        record.effects += 1;
        let id = self.next_effect;
        self.next_effect = self.next_effect.wrapping_add(1).max(1);
        self.effects.insert(
            id,
            EffectRecord {
                group,
                kind: std::mem::discriminant(&effect),
            },
        );
        let state = EffectState::new(&effect, self.sample_rate);
        let slot = EffectSlot {
            id,
            priority,
            enabled: true,
            state,
        };
        self.send(Command::AddEffect {
            group: group.0 as usize,
            slot: Box::new(slot),
        });
        Ok(EffectId(id))
    }

    /// Change an effect's parameters. Switching to a different kind of effect rebuilds it
    /// (its tail is lost).
    pub fn set_effect(&mut self, id: EffectId, effect: Effect) -> Result<(), AudioError> {
        let rate = self.sample_rate;
        let record = self
            .effects
            .get_mut(&id.0)
            .ok_or(AudioError::UnknownEffect(id))?;
        let group = record.group.0 as usize;
        let kind = std::mem::discriminant(&effect);
        if kind == record.kind {
            self.send(Command::SetEffect {
                group,
                id: id.0,
                effect,
            });
        } else {
            record.kind = kind;
            let state = Box::new(EffectState::new(&effect, rate));
            self.send(Command::ReplaceEffect {
                group,
                id: id.0,
                state,
            });
        }
        Ok(())
    }

    /// `SoundEffect.Enabled`.
    pub fn set_effect_enabled(&mut self, id: EffectId, enabled: bool) -> Result<(), AudioError> {
        let record = self
            .effects
            .get(&id.0)
            .ok_or(AudioError::UnknownEffect(id))?;
        let group = record.group.0 as usize;
        self.send(Command::SetEffectEnabled {
            group,
            id: id.0,
            enabled,
        });
        Ok(())
    }

    /// Remove an effect from its group.
    pub fn remove_effect(&mut self, id: EffectId) -> Result<(), AudioError> {
        let record = self
            .effects
            .remove(&id.0)
            .ok_or(AudioError::UnknownEffect(id))?;
        if let Some(g) = self
            .groups
            .get_mut(record.group.0 as usize)
            .and_then(Option::as_mut)
        {
            g.effects = g.effects.saturating_sub(1);
        }
        self.send(Command::RemoveEffect {
            group: record.group.0 as usize,
            id: id.0,
        });
        Ok(())
    }
}

fn record_is_playing(rec: &SoundRecord) -> bool {
    if rec.shared.applied_seq.load(Ordering::Acquire) < rec.seq {
        rec.playing_intent
    } else {
        rec.shared.state.load(Ordering::Acquire) == STATE_PLAYING
    }
}

fn record_is_paused(rec: &SoundRecord) -> bool {
    if rec.shared.applied_seq.load(Ordering::Acquire) < rec.seq {
        rec.paused_intent
    } else {
        rec.shared.state.load(Ordering::Acquire) == STATE_PAUSED
    }
}

fn clamp_finite(value: f32, min: f32, max: f32, name: &str) -> Result<f32, AudioError> {
    if value.is_finite() {
        Ok(value.clamp(min, max))
    } else {
        Err(AudioError::InvalidParameter(format!(
            "{name} must be finite, got {value}"
        )))
    }
}

/// Clamp properties to their documented ranges; non-finite numbers fall back to defaults.
fn sanitize(mut p: SoundProperties) -> SoundProperties {
    let d = SoundProperties::default();
    let fix = |v: f32, lo: f32, hi: f32, default: f32| {
        if v.is_finite() {
            v.clamp(lo, hi)
        } else {
            default
        }
    };
    p.volume = fix(p.volume, 0.0, 10.0, d.volume);
    p.playback_speed = fix(p.playback_speed, 0.0, 20.0, d.playback_speed);
    p.roll_off_min_distance = fix(
        p.roll_off_min_distance,
        0.0,
        f32::MAX,
        d.roll_off_min_distance,
    );
    p.roll_off_max_distance = fix(
        p.roll_off_max_distance,
        0.0,
        f32::MAX,
        d.roll_off_max_distance,
    )
    .max(p.roll_off_min_distance);
    p.time_position = if p.time_position.is_finite() {
        p.time_position.max(0.0)
    } else {
        0.0
    };
    if let Some(e) = p.emitter
        && !(e.position.is_finite() && e.velocity.is_finite())
    {
        p.emitter = None;
    }
    p
}
