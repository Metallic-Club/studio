//! Sound assets (#811): WAV, Ogg Vorbis, FLAC and MP3 files decoded to what the engine's mixer plays, stereo samples
//! at the file's own rate. Mono is played on both sides; more than two channels keep the first two.

use std::io::Cursor;

use thiserror::Error;

/// The extensions of the sound files a `Sound` can play (lowercase, without the dot).
pub const SOUND_EXTENSIONS: &[&str] = &["wav", "ogg", "mp3", "flac"];

/// A decoded sound: interleaved stereo samples (left, right, left, ...) from -1 to 1, `rate` frames a second.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundAsset {
    /// Frames (a left and a right sample) a second.
    pub rate: u32,
    /// How many channels the file has (1 mono, 2 stereo, ...): the samples are stereo whatever it is (#831).
    pub channels: usize,
    /// The samples, two per frame.
    pub samples: Vec<f32>,
}

impl SoundAsset {
    /// How long it plays, in seconds.
    pub fn seconds(&self) -> f64 {
        if self.rate == 0 {
            return 0.0;
        }
        (self.samples.len() / 2) as f64 / f64::from(self.rate)
    }
}

/// Why a sound file could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SoundError {
    /// Its extension is none of [`SOUND_EXTENSIONS`].
    #[error("`.{0}` is not a sound file Metallic plays (WAV, Ogg Vorbis, FLAC or MP3)")]
    Format(String),
    /// The file is damaged or not what its extension says.
    #[error("the file could not be decoded: {0}")]
    Decode(String),
    /// It decoded to nothing.
    #[error("the file holds no sound")]
    Empty,
}

/// Decodes `bytes`, a sound file with the extension `extension` (`wav`, `ogg`, `mp3` or `flac`, any case).
pub fn decode_sound(bytes: &[u8], extension: &str) -> Result<SoundAsset, SoundError> {
    let (rate, channels, samples) = match extension.to_ascii_lowercase().as_str() {
        "wav" => wav(bytes)?,
        "ogg" => ogg(bytes)?,
        "flac" => flac(bytes)?,
        "mp3" => mp3(bytes),
        other => return Err(SoundError::Format(other.to_owned())),
    };
    if rate == 0 || channels == 0 || samples.is_empty() {
        return Err(SoundError::Empty);
    }
    Ok(SoundAsset {
        rate,
        channels,
        samples: stereo(&samples, channels),
    })
}

/// `samples` with `channels` per frame as two: mono doubled, extra channels dropped.
fn stereo(samples: &[f32], channels: usize) -> Vec<f32> {
    match channels {
        2 => samples.to_vec(),
        1 => samples.iter().flat_map(|&s| [s, s]).collect(),
        _ => samples
            .chunks_exact(channels)
            .flat_map(|frame| [frame[0], frame[1]])
            .collect(),
    }
}

fn decode_error(error: impl std::fmt::Display) -> SoundError {
    SoundError::Decode(error.to_string())
}

/// Rate, channels and samples of a WAV file (integer PCM of any width, or 32-bit float).
fn wav(bytes: &[u8]) -> Result<(u32, usize, Vec<f32>), SoundError> {
    let reader = hound::WavReader::new(Cursor::new(bytes)).map_err(decode_error)?;
    let spec = reader.spec();
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(decode_error)?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|s| s as f32 * scale))
                .collect::<Result<Vec<_>, _>>()
                .map_err(decode_error)?
        }
    };
    Ok((spec.sample_rate, usize::from(spec.channels), samples))
}

/// Rate, channels and samples of an Ogg Vorbis file.
fn ogg(bytes: &[u8]) -> Result<(u32, usize, Vec<f32>), SoundError> {
    let mut reader = lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes)).map_err(decode_error)?;
    let rate = reader.ident_hdr.audio_sample_rate;
    let channels = usize::from(reader.ident_hdr.audio_channels);
    let mut samples = Vec::new();
    while let Some(packet) = reader.read_dec_packet_itl().map_err(decode_error)? {
        samples.extend(packet.into_iter().map(|s| f32::from(s) / 32768.0));
    }
    Ok((rate, channels, samples))
}

/// Rate, channels and samples of a FLAC file.
fn flac(bytes: &[u8]) -> Result<(u32, usize, Vec<f32>), SoundError> {
    let mut reader = claxon::FlacReader::new(Cursor::new(bytes)).map_err(decode_error)?;
    let info = reader.streaminfo();
    let scale = 1.0 / (1u64 << (info.bits_per_sample.clamp(1, 32) - 1)) as f32;
    let samples = reader
        .samples()
        .map(|s| s.map(|s| s as f32 * scale))
        .collect::<Result<Vec<_>, _>>()
        .map_err(decode_error)?;
    Ok((info.sample_rate, info.channels as usize, samples))
}

/// Rate, channels and samples of an MP3 (or MP1, MP2) file; nothing when no frame decodes.
fn mp3(bytes: &[u8]) -> (u32, usize, Vec<f32>) {
    let decoded = nanomp3::decode_all::<f32>(bytes);
    let channels = decoded.channels.map_or(0, |c| c as usize);
    (decoded.sample_rate, channels, decoded.samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV file of `frames` frames of 16-bit `channels`-channel PCM at `rate`, each sample `value`.
    fn wav_file(rate: u32, channels: u16, frames: usize, value: i16) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut out = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut out, spec).unwrap();
        for _ in 0..frames * usize::from(channels) {
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
        out.into_inner()
    }

    #[test]
    fn a_mono_wav_plays_on_both_sides() {
        let sound = decode_sound(&wav_file(22_050, 1, 22_050, 16_384), "WAV").unwrap();
        assert_eq!(sound.rate, 22_050);
        assert_eq!(sound.samples.len(), 2 * 22_050);
        assert!((sound.samples[0] - 0.5).abs() < 1e-4 && sound.samples[0] == sound.samples[1]);
        assert!((sound.seconds() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn what_cannot_be_decoded_says_why() {
        assert_eq!(decode_sound(b"", "aiff"), Err(SoundError::Format("aiff".into())));
        assert!(matches!(decode_sound(b"not a wav", "wav"), Err(SoundError::Decode(_))));
        assert_eq!(decode_sound(b"no frames here", "mp3"), Err(SoundError::Empty));
        assert!(matches!(
            decode_sound(b"OggS nonsense", "ogg"),
            Err(SoundError::Decode(_))
        ));
        assert!(matches!(decode_sound(b"fLaC", "flac"), Err(SoundError::Decode(_))));
    }

    #[test]
    fn five_channels_keep_the_first_two() {
        assert_eq!(
            stereo(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0], 5),
            [1.0, 2.0, 6.0, 7.0]
        );
    }
}
