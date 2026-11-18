//! [`Sounds`]: the game's `Sound`s played through the mixer (#811).
//!
//! Every frame of a running game, each sound (`metallic_classes`' `SoundList`) is brought in line with its
//! properties: a sound that is `Playing` and in the game (or played with `SoundService:PlayLocalSound`) has a voice
//! in the [`Mixer`] with its `Volume`, `PlaybackSpeed` and `Looped`, started at its `TimePosition`; any other has
//! none. What the mixer knows goes back into the sound: `TimePosition` as it plays, `IsPlaying`, and at the end
//! `Playing` false, `TimePosition` 0 and `Ended`. A script that sets `TimePosition` moves the voice there; `Play`
//! while playing starts over (its hidden play count changed).
//!
//! Sound files are read and decoded on a worker thread, once each, from the reader the game's host gives
//! ([`Sounds::set_reader`]): a sound waits until its file is here, then `TimeLength` and `IsLoaded` are set and
//! `Loaded` fires. A file that is missing or cannot be decoded is reported once, as a warning, and its sounds stay
//! silent.
//!
//! The mixer plays on a device only once the host opened one ([`Sounds::open_output`]); without (tests, servers,
//! Linux for now) everything above still happens, unheard. A paused game pauses the mixer.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use metallic_classes::audio::sound::{PLAY_COUNT, PLAY_LOCAL, SoundList, fire};
use metallic_core::{DataModel, Id, PropId, ScriptValue, Value};
use metallic_platform::audio::{AudioError, AudioOutput, Clip, Mixer, VoiceId, VoiceSettings};
use metallic_project::{AssetError, AssetReader};

/// A sound file's state.
enum ClipState {
    /// Asked for.
    Loading,
    /// Decoded.
    Ready(Clip),
    /// Missing or not a sound (reported).
    Failed,
}

/// What the engine keeps about one sound.
#[derive(Default)]
struct SoundState {
    voice: Option<VoiceId>,
    /// The `SoundId` it was last seen with.
    sound_id: Arc<str>,
    /// `TimeLength`, `IsLoaded` and `Loaded` were done for that id.
    loaded: bool,
    /// Its play count when last seen.
    play_count: i64,
    /// The `TimePosition` last written: anything else was set by a script.
    written: f64,
    /// Times its voice started over.
    loops: u32,
}

/// The Sound class's properties, found once.
#[derive(Clone, Copy)]
struct Props {
    sound_id: PropId,
    volume: PropId,
    speed: PropId,
    looped: PropId,
    playing: PropId,
    position: PropId,
    length: PropId,
    is_playing: PropId,
    is_paused: PropId,
    is_loaded: PropId,
    play_count: PropId,
    play_local: PropId,
}

impl Props {
    fn of(dm: &DataModel) -> Option<Self> {
        let registry = dm.registry();
        let class = registry.class_id(metallic_classes::audio::sound::CLASS.name)?;
        let prop = |name: &str| registry.prop_id(class, name);
        Some(Self {
            sound_id: prop("SoundId")?,
            volume: prop("Volume")?,
            speed: prop("PlaybackSpeed")?,
            looped: prop("Looped")?,
            playing: prop("Playing")?,
            position: prop("TimePosition")?,
            length: prop("TimeLength")?,
            is_playing: prop("IsPlaying")?,
            is_paused: prop("IsPaused")?,
            is_loaded: prop("IsLoaded")?,
            play_count: prop(PLAY_COUNT)?,
            play_local: prop(PLAY_LOCAL)?,
        })
    }
}

/// The worker that reads and decodes sound files.
struct Loader {
    jobs: Sender<Arc<str>>,
    loaded: Receiver<(Arc<str>, Result<Clip, AssetError>)>,
}

/// The game's sounds and the mixer that plays them (see the [module documentation](self)).
#[derive(Default)]
pub struct Sounds {
    mixer: Mixer,
    output: Option<AudioOutput>,
    loader: Option<Loader>,
    clips: HashMap<Arc<str>, ClipState>,
    states: HashMap<Id, SoundState>,
    props: Option<Props>,
    warnings: Vec<String>,
    /// Whether the mixer was paused last.
    paused: bool,
}

impl std::fmt::Debug for Sounds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sounds")
            .field("output", &self.output)
            .field("clips", &self.clips.len())
            .field("sounds", &self.states.len())
            .finish_non_exhaustive()
    }
}

impl Sounds {
    /// No device and no files yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The mixer the sounds play through (to look at what plays, or to render it in a test).
    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    /// Plays the mixer on the system's default device from now on. Fails where there is none.
    pub fn open_output(&mut self) -> Result<(), AudioError> {
        if self.output.is_none() {
            self.output = Some(AudioOutput::open(&self.mixer)?);
        }
        Ok(())
    }

    /// Reads sound files with `reader` from now on (on a worker thread). Files asked for before are asked again.
    pub fn set_reader(&mut self, reader: AssetReader) {
        let (jobs, job_queue) = mpsc::channel::<Arc<str>>();
        let (done, loaded) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("metallic-sounds".into())
            .spawn(move || {
                let mut reader = reader;
                for id in job_queue {
                    let result = reader.load_sound(&id).map(|sound| Clip {
                        rate: sound.rate,
                        samples: sound.samples.into(),
                    });
                    if done.send((id, result)).is_err() {
                        break;
                    }
                }
            });
        match spawned {
            Ok(_) => {
                self.loader = Some(Loader { jobs, loaded });
                self.clips.retain(|_, clip| matches!(clip, ClipState::Ready(_)));
            }
            Err(error) => log::warn!("sounds cannot be loaded: {error}"),
        }
    }

    /// What could not be loaded, each once.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Whether every sound file asked for is loaded (or failed).
    pub fn idle(&self) -> bool {
        !self.clips.values().any(|clip| matches!(clip, ClipState::Loading))
    }

    /// Stops every voice and forgets the sounds (the game stopped). Loaded files are kept.
    pub fn stop(&mut self) {
        self.mixer.stop_all();
        self.states.clear();
    }

    /// Brings the voices in line with the sounds (see the [module documentation](self)); `running` is false while
    /// the game is paused, which pauses the mixer.
    pub fn step(&mut self, dm: &mut DataModel, running: bool) {
        if self.paused == running {
            // Only on a change: it takes the mixer's lock.
            self.paused = !running;
            self.mixer.set_paused(!running);
        }
        if !running {
            return;
        }
        self.receive();
        // Most games have no sound: nothing more to do.
        let Some(mut list) = dm.take_state::<SoundList>() else {
            return;
        };
        list.sounds.retain(|&id| dm.contains(id));
        let ids = list.sounds.clone();
        dm.put_state(list);
        if self.props.is_none() {
            self.props = Props::of(dm);
        }
        let Some(props) = self.props else {
            return;
        };
        let gone: Vec<Id> = self.states.keys().copied().filter(|id| !dm.contains(*id)).collect();
        for id in gone {
            if let Some(voice) = self.states.remove(&id).and_then(|state| state.voice) {
                self.mixer.stop(voice);
            }
        }
        let root = dm.root();
        for id in ids {
            self.sync(dm, id, props, root);
        }
    }

    /// Takes in the files the worker finished.
    fn receive(&mut self) {
        let Some(loader) = &self.loader else {
            return;
        };
        while let Ok((id, result)) = loader.loaded.try_recv() {
            let state = match result {
                Ok(clip) => ClipState::Ready(clip),
                Err(error) => {
                    let warning = format!("sound {id}: {error}");
                    log::warn!("{warning}");
                    self.warnings.push(warning);
                    ClipState::Failed
                }
            };
            self.clips.insert(id, state);
        }
    }

    /// The clip of `sound_id`, asking the worker for it the first time.
    fn clip(&mut self, sound_id: &Arc<str>) -> Option<Clip> {
        if sound_id.is_empty() {
            return None;
        }
        match self.clips.get(sound_id) {
            Some(ClipState::Ready(clip)) => Some(clip.clone()),
            Some(_) => None,
            None => {
                if let Some(loader) = &self.loader
                    && loader.jobs.send(Arc::clone(sound_id)).is_ok()
                {
                    self.clips.insert(Arc::clone(sound_id), ClipState::Loading);
                }
                None
            }
        }
    }

    /// One sound: its voice made, changed or stopped, and what the mixer knows written back.
    fn sync(&mut self, dm: &mut DataModel, id: Id, p: Props, root: Option<Id>) {
        let number = |dm: &DataModel, prop| dm.number(id, prop).unwrap_or(0.0);
        let flag = |dm: &DataModel, prop| dm.bool(id, prop).unwrap_or(false);
        let sound_id: Arc<str> = match dm.get(id, p.sound_id) {
            Ok(Value::String(s)) => s,
            _ => Arc::from(""),
        };
        let here = flag(dm, p.play_local) || root.is_some_and(|root| dm.is_descendant_of(id, root).unwrap_or(false));
        let want = here && flag(dm, p.playing);
        let mut state = self.states.remove(&id).unwrap_or_default();
        if state.sound_id != sound_id {
            if let Some(voice) = state.voice.take() {
                self.mixer.stop(voice);
            }
            state.sound_id = Arc::clone(&sound_id);
            state.loaded = false;
        }
        let clip = self.clip(&sound_id);
        if let Some(clip) = &clip
            && !state.loaded
        {
            state.loaded = true;
            write(dm, id, p.length, Value::Number(clip.seconds()));
            write(dm, id, p.is_loaded, Value::Bool(true));
            fire(dm, id, "Loaded");
        }
        let play_count = match dm.get(id, p.play_count) {
            Ok(Value::Int(count)) => count,
            _ => 0,
        };
        let position = number(dm, p.position);
        match (want, clip) {
            (true, Some(clip)) => {
                let settings = VoiceSettings {
                    volume: number(dm, p.volume) as f32,
                    speed: number(dm, p.speed) as f32,
                    looped: flag(dm, p.looped),
                };
                let voice = match state.voice {
                    Some(voice) => {
                        self.mixer.set(voice, settings);
                        if play_count != state.play_count || (position - state.written).abs() > 1e-9 {
                            self.mixer.seek(voice, position);
                        }
                        voice
                    }
                    None => {
                        let start = if position >= clip.seconds() { 0.0 } else { position };
                        state.loops = 0;
                        self.mixer.play(clip, settings, start)
                    }
                };
                state.voice = Some(voice);
                if let Some(status) = self.mixer.status(voice) {
                    if status.loops > state.loops {
                        state.loops = status.loops;
                        did_loop(dm, id, &sound_id, status.loops);
                    }
                    if status.finished {
                        self.mixer.stop(voice);
                        state.voice = None;
                        state.written = 0.0;
                        write(dm, id, p.playing, Value::Bool(false));
                        write(dm, id, p.position, Value::Number(0.0));
                        write(dm, id, p.is_playing, Value::Bool(false));
                        write(dm, id, p.is_paused, Value::Bool(false));
                        fire(dm, id, "Ended");
                    } else {
                        state.written = status.seconds;
                        write(dm, id, p.position, Value::Number(status.seconds));
                        write(dm, id, p.is_playing, Value::Bool(true));
                    }
                }
            }
            _ => {
                if let Some(voice) = state.voice.take() {
                    self.mixer.stop(voice);
                }
                state.written = position;
                write(dm, id, p.is_playing, Value::Bool(false));
            }
        }
        state.play_count = play_count;
        self.states.insert(id, state);
    }
}

/// Sets `prop` of `id` to `value` unless it has it already, so an unchanged sound marks nothing.
fn write(dm: &mut DataModel, id: Id, prop: PropId, value: Value) {
    if dm.get(id, prop).is_ok_and(|current| current == value) {
        return;
    }
    // Only a destroyed sound fails, and nothing reads it then.
    let _ = dm.set(id, prop, value);
}

/// Fires `DidLoop` with the sound's id and how many times it looped.
fn did_loop(dm: &mut DataModel, id: Id, sound_id: &Arc<str>, loops: u32) {
    if let Ok(signal) = dm.signal_id(id, "DidLoop") {
        let _ = dm.fire(
            id,
            signal,
            vec![
                ScriptValue::String(Arc::clone(sound_id)),
                ScriptValue::Number(f64::from(loops)),
            ],
        );
    }
}
