//! The sound preview (#831): a project sound in a tab of its own, `sound:<asset id>`, opened by a double-click on it in
//! the Assets panel. It decodes the file once, keeps its waveform's outline, and plays it through a mixer of its own
//! on the system's device, opened the first time Play is pressed (never in tests). One sound plays at a time.

use std::collections::HashMap;
use std::sync::Arc;

use metallic_engine::metallic_platform::audio::{AudioOutput, Clip, Mixer, VoiceId, VoiceSettings};
use metallic_engine::metallic_project::AssetId;

use super::EditorState;

/// The sound preview's panel id, the first part of its tab's id.
pub const PANEL: &str = "sound";

/// How many columns a waveform's outline has: more than any tab is wide.
pub const WAVEFORM_COLUMNS: usize = 2048;

/// The tab id of the sound `id`'s preview.
pub fn pane(id: AssetId) -> String {
    format!("{PANEL}:{id}")
}

/// The sound a sound tab's id names.
pub fn sound_of(pane: &str) -> Option<AssetId> {
    pane.strip_prefix(PANEL)?.strip_prefix(':')?.parse().ok()
}

/// A sound as its preview shows it.
#[derive(Debug)]
pub struct Preview {
    /// Frames a second.
    pub rate: u32,
    /// How many channels the file has.
    pub channels: usize,
    /// How long it plays, in seconds.
    pub seconds: f64,
    /// The waveform's outline: for each of [`WAVEFORM_COLUMNS`] stretches of the sound, the lowest and highest sample
    /// of its left and right mixed, from -1 to 1.
    pub outline: Vec<[f32; 2]>,
    clip: Clip,
}

impl Preview {
    /// The preview of a decoded sound: `samples` stereo, `rate` frames a second.
    pub fn new(rate: u32, channels: usize, samples: Vec<f32>) -> Self {
        let frames = samples.len() / 2;
        let columns = WAVEFORM_COLUMNS.min(frames.max(1));
        let outline = (0..columns)
            .map(|column| {
                let from = column * frames / columns;
                let to = ((column + 1) * frames / columns).max(from + 1).min(frames);
                samples
                    .get(from * 2..to * 2)
                    .unwrap_or_default()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|[left, right]| (left + right) * 0.5)
                    .fold([0.0f32, 0.0], |[low, high], s| [low.min(s), high.max(s)])
            })
            .collect();
        let seconds = if rate == 0 {
            0.0
        } else {
            frames as f64 / f64::from(rate)
        };
        Self {
            rate,
            channels,
            seconds,
            outline,
            clip: Clip {
                rate,
                samples: samples.into(),
            },
        }
    }
}

/// The previews read, the tabs asked for, and what plays.
#[derive(Debug, Default)]
pub struct SoundPreviews {
    /// Each sound read, `None` for one that could not be.
    shown: HashMap<AssetId, Option<Arc<Preview>>>,
    opening: Vec<AssetId>,
    mixer: Mixer,
    output: Option<AudioOutput>,
    /// The sound playing, and its voice.
    playing: Option<(AssetId, VoiceId)>,
    /// Where each sound not playing stands, in seconds.
    places: HashMap<AssetId, f64>,
}

impl SoundPreviews {
    /// The tabs asked for since the last call.
    pub fn take_opening(&mut self) -> Vec<AssetId> {
        std::mem::take(&mut self.opening)
    }

    /// The mixer the previews play through (to render it by hand in a test).
    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }

    /// Whether `id` is playing.
    pub fn is_playing(&self, id: AssetId) -> bool {
        self.playing.is_some_and(|(playing, _)| playing == id)
    }

    /// Where `id` is, in seconds.
    pub fn place(&self, id: AssetId) -> f64 {
        match self.playing {
            Some((playing, voice)) if playing == id => self.mixer.status(voice).map_or(0.0, |status| status.seconds),
            _ => self.places.get(&id).copied().unwrap_or(0.0),
        }
    }

    /// Plays `preview` (the sound `id`) from where it stands, or from the start when it stands at its end. Any other
    /// sound stops where it is.
    pub fn play(&mut self, id: AssetId, preview: &Preview) {
        self.pause();
        if self.output.is_none() && !cfg!(test) {
            match AudioOutput::open(&self.mixer) {
                Ok(output) => self.output = Some(output),
                Err(error) => log::info!("sound preview: plays unheard: {error}"),
            }
        }
        let mut from = self.places.get(&id).copied().unwrap_or(0.0);
        if from >= preview.seconds {
            from = 0.0;
        }
        let voice = self.mixer.play(preview.clip.clone(), VoiceSettings::default(), from);
        self.playing = Some((id, voice));
    }

    /// Stops what plays where it is.
    pub fn pause(&mut self) {
        if let Some((id, voice)) = self.playing.take() {
            let at = self.mixer.status(voice).map_or(0.0, |status| status.seconds);
            self.places.insert(id, at);
            self.mixer.stop(voice);
        }
    }

    /// Moves `id` to `seconds` in, playing on from there if it plays.
    pub fn seek(&mut self, id: AssetId, seconds: f64) {
        match self.playing {
            Some((playing, voice)) if playing == id => self.mixer.seek(voice, seconds),
            _ => {
                self.places.insert(id, seconds.max(0.0));
            }
        }
    }

    /// Once a frame: a sound played to its end stops, back at its start.
    pub fn update(&mut self) {
        if let Some((id, voice)) = self.playing
            && self.mixer.status(voice).is_none_or(|status| status.finished)
        {
            self.mixer.stop(voice);
            self.playing = None;
            self.places.insert(id, 0.0);
        }
    }

    /// Forgets `id`'s preview (its file changed or left), stopping it if it plays.
    pub fn forget(&mut self, id: AssetId) {
        if self.is_playing(id) {
            self.pause();
        }
        self.shown.remove(&id);
        self.places.remove(&id);
    }
}

impl EditorState {
    /// Opens the sound at `path` in its preview tab (#831).
    pub fn open_sound(&mut self, path: &str) -> Result<(), String> {
        let id = self
            .project
            .assets()
            .id(path)
            .ok_or_else(|| format!("{path} has no asset id yet."))?;
        self.sounds.opening.push(id);
        Ok(())
    }

    /// The sound `id` as its preview shows it, decoded the first time.
    pub fn sound_preview(&mut self, id: AssetId) -> Option<Arc<Preview>> {
        if let Some(shown) = self.sounds.shown.get(&id) {
            return shown.clone();
        }
        let shown = self
            .project
            .asset_reader()
            .load_sound(&id.to_string())
            .ok()
            .map(|sound| Arc::new(Preview::new(sound.rate, sound.channels, sound.samples)));
        self.sounds.shown.insert(id, shown.clone());
        shown
    }

    /// How many `Sound`s in the game play `id`.
    pub fn sounds_using(&self, id: AssetId) -> usize {
        let Some(root) = self.dm.root() else {
            return 0;
        };
        let id = id.to_string();
        let Ok(descendants) = self.dm.descendants(root) else {
            return 0;
        };
        descendants
            .filter(|&instance| {
                self.dm.class_name(instance) == Ok("Sound")
                    && matches!(self.dm.get_by_name(instance, "SoundId"), Ok(metallic_engine::metallic_core::Value::String(s)) if *s == *id)
            })
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waveform_s_outline_holds_each_stretch_s_lowest_and_highest() {
        // One second at 4 frames: up, down, silent, loud.
        let samples = vec![0.5, 0.5, -0.5, -0.5, 0.0, 0.0, 1.0, 0.0];
        let preview = Preview::new(4, 1, samples);
        assert_eq!(preview.outline, [[0.0, 0.5], [-0.5, 0.0], [0.0, 0.0], [0.0, 0.5]]);
        assert!((preview.seconds - 1.0).abs() < 1e-9);
        assert_eq!(sound_of(&pane(AssetId::from_u128(7))), Some(AssetId::from_u128(7)));
    }

    #[test]
    fn a_preview_plays_pauses_seeks_and_stops_at_its_end() {
        let id = AssetId::from_u128(1);
        let preview = Preview::new(4, 2, vec![0.25; 16]);
        let mut sounds = SoundPreviews::default();
        sounds.play(id, &preview);
        assert!(sounds.is_playing(id));
        let mut out = [0.0; 4];
        sounds.mixer().render(&mut out, 2, 4);
        assert!((sounds.place(id) - 0.5).abs() < 1e-9);
        sounds.pause();
        assert!(!sounds.is_playing(id));
        assert!((sounds.place(id) - 0.5).abs() < 1e-9, "the place is kept");
        sounds.seek(id, 1.5);
        sounds.play(id, &preview);
        let mut rest = [0.0; 8];
        sounds.mixer().render(&mut rest, 2, 4);
        sounds.update();
        assert!(!sounds.is_playing(id), "it stops at its end");
        assert_eq!(sounds.place(id), 0.0, "back at the start");
    }
}
