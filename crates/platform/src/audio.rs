//! Sound output (#811): a [`Mixer`] of playing voices, and the [`AudioOutput`] that plays it on the system's default
//! device.
//!
//! The engine plays decoded clips ([`Clip`], stereo samples at their own rate) through the mixer from the main thread:
//! each voice has a volume, a speed (which also changes the pitch) and loops or not, and the engine reads back where
//! each one is. The device's thread calls [`Mixer::render`] for every buffer it needs, which adds the voices up,
//! resampled to the device's rate. The two share the voices behind a mutex that each side holds only for a few
//! microseconds.
//!
//! Output uses cpal: WASAPI on Windows, Core Audio on macOS and iOS, AAudio on Android. Linux has no output yet (cpal
//! links ALSA there, a system package every Linux build would need), nor does a build without the `audio` feature:
//! [`AudioOutput::open`] then says [`AudioError::Unsupported`], and the mixer still works, unheard.

use std::sync::{Arc, Mutex, MutexGuard};

use thiserror::Error;

/// A decoded sound: interleaved stereo samples (left, right, ...) from -1 to 1, `rate` frames a second.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// Frames a second.
    pub rate: u32,
    /// Two samples a frame.
    pub samples: Arc<[f32]>,
}

impl Clip {
    /// How many frames it has.
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// How long it plays at speed 1, in seconds.
    pub fn seconds(&self) -> f64 {
        if self.rate == 0 {
            0.0
        } else {
            self.frames() as f64 / f64::from(self.rate)
        }
    }

    /// The frame at `index` (zero past the end).
    fn frame(&self, index: usize) -> (f32, f32) {
        match self.samples.get(index * 2..index * 2 + 2) {
            Some([left, right]) => (*left, *right),
            _ => (0.0, 0.0),
        }
    }
}

/// A playing voice's handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoiceId(u64);

/// How a voice plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceSettings {
    /// Loudness: 1 as recorded, 0 silent (above 1 is louder, clipped at the device).
    pub volume: f32,
    /// How fast: 1 as recorded, 2 twice as fast and an octave higher.
    pub speed: f32,
    /// Whether it starts over at the end instead of finishing.
    pub looped: bool,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            speed: 1.0,
            looped: false,
        }
    }
}

/// Where a voice is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceStatus {
    /// Seconds into the clip.
    pub seconds: f64,
    /// It reached the end (and does not loop).
    pub finished: bool,
    /// How many times it started over.
    pub loops: u32,
}

/// One playing clip.
#[derive(Debug)]
struct Voice {
    id: VoiceId,
    clip: Clip,
    settings: VoiceSettings,
    /// In the clip's frames.
    position: f64,
    loops: u32,
    finished: bool,
}

#[derive(Debug, Default)]
struct State {
    voices: Vec<Voice>,
    next: u64,
    /// While set, every voice stands still and the output is silent.
    paused: bool,
    /// Frames rendered so far: whether a device is pulling sound at all (#830).
    rendered: u64,
}

/// The voices and what plays them (see the [module documentation](self)). Clones share the voices.
#[derive(Clone, Debug, Default)]
pub struct Mixer {
    state: Arc<Mutex<State>>,
}

impl Mixer {
    /// A mixer with no voices.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic elsewhere while holding it leaves only plain numbers behind: still fine to use.
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Starts `clip` playing `seconds` in, as `settings` say.
    pub fn play(&self, clip: Clip, settings: VoiceSettings, seconds: f64) -> VoiceId {
        let mut state = self.lock();
        let id = VoiceId(state.next);
        state.next += 1;
        let position = (seconds.max(0.0) * f64::from(clip.rate)).min(clip.frames() as f64);
        state.voices.push(Voice {
            id,
            clip,
            settings,
            position,
            loops: 0,
            finished: false,
        });
        id
    }

    /// Changes how `voice` plays. Does nothing once it was stopped.
    pub fn set(&self, voice: VoiceId, settings: VoiceSettings) {
        if let Some(found) = self.lock().voices.iter_mut().find(|v| v.id == voice) {
            found.settings = settings;
        }
    }

    /// Moves `voice` to `seconds` into its clip.
    pub fn seek(&self, voice: VoiceId, seconds: f64) {
        if let Some(found) = self.lock().voices.iter_mut().find(|v| v.id == voice) {
            found.position = (seconds.max(0.0) * f64::from(found.clip.rate)).min(found.clip.frames() as f64);
            found.finished = false;
        }
    }

    /// Stops `voice` and forgets it.
    pub fn stop(&self, voice: VoiceId) {
        self.lock().voices.retain(|v| v.id != voice);
    }

    /// Stops every voice.
    pub fn stop_all(&self) {
        self.lock().voices.clear();
    }

    /// Holds every voice where it is (a paused game), or lets them go on.
    pub fn set_paused(&self, paused: bool) {
        self.lock().paused = paused;
    }

    /// Where `voice` is; `None` once it was stopped.
    pub fn status(&self, voice: VoiceId) -> Option<VoiceStatus> {
        self.lock().voices.iter().find(|v| v.id == voice).map(|v| VoiceStatus {
            seconds: if v.clip.rate == 0 {
                0.0
            } else {
                v.position / f64::from(v.clip.rate)
            },
            finished: v.finished,
            loops: v.loops,
        })
    }

    /// How many frames were rendered so far, by a device or by hand.
    pub fn rendered_frames(&self) -> u64 {
        self.lock().rendered
    }

    /// How many voices are playing or finished but not stopped yet.
    pub fn voice_count(&self) -> usize {
        self.lock().voices.len()
    }

    /// Fills `out`, `channels` interleaved samples a frame at `rate` frames a second, with every voice added up:
    /// each resampled from its clip's rate (linearly) and its speed, then moved on. Stereo goes to the first two
    /// channels (mono gets their mean); others are silent.
    pub fn render(&self, out: &mut [f32], channels: usize, rate: u32) {
        out.fill(0.0);
        if channels == 0 || rate == 0 {
            return;
        }
        let mut state = self.lock();
        state.rendered += (out.len() / channels) as u64;
        if state.paused {
            return;
        }
        for voice in &mut state.voices {
            if voice.finished {
                continue;
            }
            let frames = voice.clip.frames();
            if frames == 0 {
                voice.finished = true;
                continue;
            }
            let step = f64::from(voice.clip.rate) / f64::from(rate) * f64::from(voice.settings.speed.max(0.0));
            let volume = voice.settings.volume.max(0.0);
            for frame in out.chunks_exact_mut(channels) {
                if voice.position >= frames as f64 {
                    if voice.settings.looped {
                        voice.position %= frames as f64;
                        voice.loops += 1;
                    } else {
                        voice.position = frames as f64;
                        voice.finished = true;
                        break;
                    }
                }
                let at = voice.position.floor();
                let t = (voice.position - at) as f32;
                let index = at as usize;
                let (l0, r0) = voice.clip.frame(index);
                let next = if index + 1 < frames {
                    index + 1
                } else if voice.settings.looped {
                    0
                } else {
                    index
                };
                let (l1, r1) = voice.clip.frame(next);
                let left = (l0 + (l1 - l0) * t) * volume;
                let right = (r0 + (r1 - r0) * t) * volume;
                match frame {
                    [only] => *only += (left + right) * 0.5,
                    [l, r, ..] => {
                        *l += left;
                        *r += right;
                    }
                    [] => {}
                }
                voice.position += step;
            }
        }
        drop(state);
        for sample in out.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }
}

/// Why sound cannot be played.
#[derive(Debug, Error)]
pub enum AudioError {
    /// This build or platform has no sound output.
    #[error("this platform has no sound output yet")]
    Unsupported,
    /// The system has no output device.
    #[error("there is no sound output device")]
    NoDevice,
    /// The device refused.
    #[error("the sound device could not be opened: {0}")]
    Device(String),
}

/// The mixer playing on the system's default device, for as long as it lives.
pub struct AudioOutput {
    #[cfg(all(feature = "audio", not(target_os = "linux")))]
    _stream: cpal::Stream,
    /// The device's frames a second.
    rate: u32,
}

impl std::fmt::Debug for AudioOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioOutput")
            .field("rate", &self.rate)
            .finish_non_exhaustive()
    }
}

impl AudioOutput {
    /// Whether this build plays sound on a device at all: the `audio` feature, on a platform with output (#830).
    pub const SUPPORTED: bool = cfg!(all(feature = "audio", not(target_os = "linux")));

    /// The device's frames a second.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Plays `mixer` on the default output device.
    #[cfg(all(feature = "audio", not(target_os = "linux")))]
    pub fn open(mixer: &Mixer) -> Result<Self, AudioError> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let device = cpal::default_host()
            .default_output_device()
            .ok_or(AudioError::NoDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|e| AudioError::Device(e.to_string()))?;
        let config = supported.config();
        let channels = usize::from(config.channels);
        let rate = config.sample_rate;
        let error = |e: cpal::Error| log::warn!("sound output: {e}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => {
                let mixer = mixer.clone();
                device.build_output_stream(
                    config,
                    move |out: &mut [f32], _: &cpal::OutputCallbackInfo| mixer.render(out, channels, rate),
                    error,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let mixer = mixer.clone();
                let mut buffer = Vec::new();
                device.build_output_stream(
                    config,
                    move |out: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        buffer.resize(out.len(), 0.0);
                        mixer.render(&mut buffer, channels, rate);
                        for (o, s) in out.iter_mut().zip(&buffer) {
                            *o = (s * f32::from(i16::MAX)) as i16;
                        }
                    },
                    error,
                    None,
                )
            }
            other => {
                return Err(AudioError::Device(format!(
                    "its sample format {other:?} is not supported"
                )));
            }
        }
        .map_err(|e| AudioError::Device(e.to_string()))?;
        stream.play().map_err(|e| AudioError::Device(e.to_string()))?;
        Ok(Self { _stream: stream, rate })
    }

    /// Plays `mixer` on the default output device: there is none on this platform or build yet.
    #[cfg(not(all(feature = "audio", not(target_os = "linux"))))]
    pub fn open(mixer: &Mixer) -> Result<Self, AudioError> {
        let _ = mixer;
        Err(AudioError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(rate: u32, frames: &[(f32, f32)]) -> Clip {
        Clip {
            rate,
            samples: frames.iter().flat_map(|&(l, r)| [l, r]).collect::<Vec<_>>().into(),
        }
    }

    #[test]
    fn voices_add_up_and_finish() {
        let mixer = Mixer::new();
        let a = mixer.play(clip(4, &[(0.25, 0.5); 4]), VoiceSettings::default(), 0.0);
        let half = VoiceSettings {
            volume: 0.5,
            ..VoiceSettings::default()
        };
        mixer.play(clip(4, &[(0.5, 0.5); 2]), half, 0.0);
        let mut out = [0.0; 12];
        mixer.render(&mut out, 2, 4);
        assert_eq!(out, [0.5, 0.75, 0.5, 0.75, 0.25, 0.5, 0.25, 0.5, 0.0, 0.0, 0.0, 0.0]);
        let status = mixer.status(a).unwrap();
        assert!(status.finished);
        assert!((status.seconds - 1.0).abs() < 1e-9);
        mixer.stop(a);
        assert_eq!(mixer.status(a), None);
        assert_eq!(mixer.voice_count(), 1);
    }

    #[test]
    fn speed_and_the_device_rate_resample_and_loops_start_over() {
        let mixer = Mixer::new();
        let ramp = clip(2, &[(0.0, 0.0), (1.0, 1.0)]);
        let looped = VoiceSettings {
            looped: true,
            ..VoiceSettings::default()
        };
        let voice = mixer.play(ramp, looped, 0.0);
        // The device runs at twice the clip's rate: every other frame is between two of the clip's.
        let mut out = [0.0; 1];
        let mut lefts = Vec::new();
        for _ in 0..6 {
            mixer.render(&mut out, 1, 4);
            lefts.push(out[0]);
        }
        assert_eq!(lefts, [0.0, 0.5, 1.0, 0.5, 0.0, 0.5]);
        assert_eq!(mixer.status(voice).unwrap().loops, 1);

        // Twice as fast, from half a second in.
        let fast = VoiceSettings {
            speed: 2.0,
            ..VoiceSettings::default()
        };
        mixer.stop_all();
        let voice = mixer.play(clip(4, &[(0.1, 0.1), (0.2, 0.2), (0.3, 0.3), (0.4, 0.4)]), fast, 0.5);
        let mut out = [0.0; 1];
        mixer.render(&mut out, 1, 4);
        assert!((out[0] - 0.3).abs() < 1e-6);
        assert!((mixer.status(voice).unwrap().seconds - 1.0).abs() < 1e-9);
    }

    /// #830: the default device pulls frames from the mixer. Run by hand on a machine with sound:
    /// `cargo test -p metallic_platform --features audio --lib -- --ignored the_device_pulls`.
    #[test]
    #[ignore = "opens the system's sound device"]
    fn the_device_pulls_frames_from_the_mixer() {
        let mixer = Mixer::new();
        // Silent: a test makes no noise.
        let quiet = VoiceSettings {
            volume: 0.0,
            ..VoiceSettings::default()
        };
        mixer.play(clip(48_000, &[(1.0, 1.0); 48_000]), quiet, 0.0);
        let output = AudioOutput::open(&mixer).expect("a sound device");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while mixer.rendered_frames() < u64::from(output.rate()) / 10 {
            assert!(std::time::Instant::now() < until, "the device never asked for sound");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn a_paused_mixer_is_silent_and_holds_its_voices() {
        let mixer = Mixer::new();
        let voice = mixer.play(clip(4, &[(1.0, 1.0); 8]), VoiceSettings::default(), 0.0);
        mixer.set_paused(true);
        let mut out = [1.0; 4];
        mixer.render(&mut out, 2, 4);
        assert_eq!(out, [0.0; 4]);
        assert_eq!(mixer.status(voice).unwrap().seconds, 0.0);
        mixer.set_paused(false);
        mixer.seek(voice, 1.0);
        mixer.render(&mut out, 2, 4);
        assert!((mixer.status(voice).unwrap().seconds - 1.5).abs() < 1e-9);
    }
}
