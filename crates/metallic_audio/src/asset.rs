//! Sound assets: decoded PCM in memory, and streaming sources for long files and live audio.
//!
//! Decoding uses Symphonia (MPL-2.0, pure Rust): WAV (PCM/ADPCM), OGG Vorbis, FLAC and MP3.
//! Everything is converted to `f32` and to mono or stereo; files with more channels are folded to
//! stereo with ITU-style coefficients (centre and surrounds at -3 dB, LFE dropped), normalised so
//! a full-scale signal cannot clip.

use crate::error::AudioError;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase, Timestamp};

/// A fully decoded sound held in memory: short effects, UI clicks, loops.
///
/// Cheap to share: the samples sit behind an `Arc`, so any number of sounds can play one asset.
#[derive(Debug, Clone)]
pub struct SoundAsset {
    samples: Arc<[f32]>,
    channels: u16,
    sample_rate: u32,
    frames: usize,
}

impl SoundAsset {
    /// Build an asset from interleaved `f32` samples. More than two channels are folded to
    /// stereo. Fails for zero channels, a zero sample rate or a length that is not a whole
    /// number of frames.
    pub fn from_interleaved(
        samples: Vec<f32>,
        channels: u16,
        sample_rate: u32,
    ) -> Result<Self, AudioError> {
        if channels == 0 {
            return Err(AudioError::InvalidParameter(
                "asset has zero channels".into(),
            ));
        }
        if sample_rate == 0 {
            return Err(AudioError::InvalidParameter(
                "asset has a zero sample rate".into(),
            ));
        }
        if !samples.len().is_multiple_of(channels as usize) {
            return Err(AudioError::InvalidParameter(format!(
                "{} samples is not a whole number of {channels}-channel frames",
                samples.len()
            )));
        }
        let (samples, channels) = if channels > 2 {
            let mut out = Vec::with_capacity(samples.len() / channels as usize * 2);
            fold_to_stereo(&samples, channels as usize, &mut out);
            (out, 2)
        } else {
            (samples, channels)
        };
        let frames = samples.len() / channels as usize;
        Ok(Self {
            samples: samples.into(),
            channels,
            sample_rate,
            frames,
        })
    }

    /// A mono asset from samples.
    pub fn from_mono(samples: Vec<f32>, sample_rate: u32) -> Result<Self, AudioError> {
        Self::from_interleaved(samples, 1, sample_rate)
    }

    /// Decode a whole encoded file (WAV, OGG Vorbis, FLAC, MP3) from memory. `extension` (such as
    /// `"ogg"`) is an optional hint; the content is probed either way.
    pub fn decode(bytes: &[u8], extension: Option<&str>) -> Result<Self, AudioError> {
        let shared: Arc<[u8]> = Arc::from(bytes);
        let stream = DecoderStream::from_bytes(shared, extension)?;
        Self::decode_stream(stream)
    }

    /// Decode a whole file from disk; the extension is used as the format hint.
    pub fn decode_file(path: &Path) -> Result<Self, AudioError> {
        let stream = DecoderStream::open(path)?;
        Self::decode_stream(stream)
    }

    fn decode_stream(mut stream: DecoderStream) -> Result<Self, AudioError> {
        let channels = stream.channels();
        let rate = stream.sample_rate();
        let mut all = Vec::new();
        if let Some(frames) = stream.length_frames() {
            all.reserve(frames as usize * channels as usize);
        }
        let mut buf = vec![0.0f32; 4096 * channels as usize];
        loop {
            let n = stream.read(&mut buf)?;
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n * channels as usize]);
        }
        Self::from_interleaved(all, channels, rate)
    }

    /// Channels after folding: 1 (mono) or 2 (stereo).
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Native sample rate in Hz (the mixer resamples on the fly).
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Length in frames.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Length in seconds (Roblox `TimeLength`).
    pub fn duration_secs(&self) -> f64 {
        self.frames as f64 / self.sample_rate as f64
    }

    /// Interleaved samples (`channels()` per frame).
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// One frame as a stereo pair (mono is duplicated); silence past the end.
    #[inline]
    pub(crate) fn frame(&self, index: u64) -> [f32; 2] {
        let i = index as usize;
        if i >= self.frames {
            return [0.0; 2];
        }
        if self.channels == 1 {
            let s = self.samples[i];
            [s, s]
        } else {
            [self.samples[2 * i], self.samples[2 * i + 1]]
        }
    }
}

/// A source of PCM that is produced while it plays: streamed music, procedural audio, and from
/// milestone M2 decoded voice-chat packets. The mixer pulls from it on the audio thread, so
/// `read` should be quick and must not block.
pub trait StreamSource: Send {
    /// 1 (mono) or 2 (stereo).
    fn channels(&self) -> u16;
    /// Sample rate in Hz.
    fn sample_rate(&self) -> u32;
    /// Total length in frames when known up front.
    fn length_frames(&self) -> Option<u64>;
    /// Fill `out` with interleaved frames; return how many frames were written. 0 means the end
    /// was reached. `out.len()` is a multiple of `channels()`.
    fn read(&mut self, out: &mut [f32]) -> Result<usize, AudioError>;
    /// Jump to `frame`. Sources that cannot seek return an error; the voice then goes silent
    /// until it is stopped.
    fn seek(&mut self, frame: u64) -> Result<(), AudioError>;
}

/// Streaming decoder over an encoded file: only a packet at a time is decoded, so a long music
/// track costs kilobytes instead of tens of megabytes.
pub struct DecoderStream {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    time_base: Option<TimeBase>,
    channels: u16,
    sample_rate: u32,
    length: Option<u64>,
    /// Decoded, folded samples not yet handed out.
    pending: Vec<f32>,
    pending_pos: usize,
    /// Interleaved scratch in the source channel count.
    scratch: Vec<f32>,
    /// Frames to drop after an inexact seek.
    skip_frames: u64,
    finished: bool,
}

impl std::fmt::Debug for DecoderStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecoderStream")
            .field("channels", &self.channels)
            .field("sample_rate", &self.sample_rate)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}

impl DecoderStream {
    /// Stream from encoded bytes in memory (shared, so many sounds can stream one file).
    pub fn from_bytes(bytes: Arc<[u8]>, extension: Option<&str>) -> Result<Self, AudioError> {
        Self::new(Box::new(Cursor::new(bytes)), extension)
    }

    /// Stream from a file on disk.
    pub fn open(path: &Path) -> Result<Self, AudioError> {
        let file = std::fs::File::open(path)?;
        let ext = path.extension().and_then(|e| e.to_str());
        Self::new(Box::new(file), ext)
    }

    fn new(source: Box<dyn MediaSource>, extension: Option<&str>) -> Result<Self, AudioError> {
        let mss = MediaSourceStream::new(source, Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = extension {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| AudioError::Unsupported(e.to_string()))?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| AudioError::Unsupported("no audio track".into()))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| AudioError::Unsupported("track has no audio codec parameters".into()))?;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|e| AudioError::Unsupported(e.to_string()))?;
        let sample_rate = params
            .sample_rate
            .filter(|r| *r > 0)
            .ok_or_else(|| AudioError::Decode("unknown sample rate".into()))?;
        let source_channels = params
            .channels
            .as_ref()
            .map(|c| c.count())
            .filter(|c| *c > 0)
            .ok_or_else(|| AudioError::Decode("unknown channel count".into()))?;
        let track_id = track.id;
        let time_base = track.time_base;
        let length = track.num_frames;
        Ok(Self {
            format,
            decoder,
            track_id,
            time_base,
            channels: if source_channels == 1 { 1 } else { 2 },
            sample_rate,
            length,
            pending: Vec::new(),
            pending_pos: 0,
            scratch: Vec::new(),
            skip_frames: 0,
            finished: false,
        })
    }

    /// Decode the next packet into `pending`. Returns false at the end of the stream.
    fn decode_next(&mut self) -> Result<bool, AudioError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) | Err(SymphoniaError::ResetRequired) => return Ok(false),
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(false);
                }
                Err(e) => return Err(AudioError::Decode(e.to_string())),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(buf) => buf,
                // A corrupt packet is skipped, like every player does.
                Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
                Err(e) => return Err(AudioError::Decode(e.to_string())),
            };
            let channels = decoded.spec().channels().count().max(1);
            self.scratch.resize(decoded.samples_interleaved(), 0.0);
            decoded.copy_to_slice_interleaved(&mut self.scratch[..]);
            self.pending.clear();
            self.pending_pos = 0;
            match (channels, self.channels) {
                (1, 1) | (2, 2) => self.pending.extend_from_slice(&self.scratch),
                (1, _) => {
                    // Mono packet in a stereo stream (rare): duplicate.
                    for &s in &self.scratch {
                        self.pending.push(s);
                        self.pending.push(s);
                    }
                }
                (2, _) => {
                    // Stereo packet in a stream declared mono (rare): average.
                    self.pending.extend(
                        self.scratch
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|f| 0.5 * (f[0] + f[1])),
                    );
                }
                (_, 2) => fold_to_stereo(&self.scratch, channels, &mut self.pending),
                _ => {
                    fold_to_stereo(&self.scratch, channels, &mut self.pending);
                    let stereo = std::mem::take(&mut self.pending);
                    self.pending.extend(
                        stereo
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|f| 0.5 * (f[0] + f[1])),
                    );
                }
            }
            if self.skip_frames > 0 {
                let frames = (self.pending.len() / self.channels as usize) as u64;
                let skip = self.skip_frames.min(frames);
                self.pending_pos = (skip as usize) * self.channels as usize;
                self.skip_frames -= skip;
            }
            if self.pending_pos < self.pending.len() {
                return Ok(true);
            }
        }
    }

    fn ts_to_frames(&self, ts: Timestamp) -> u64 {
        match self.time_base {
            Some(tb) => tb
                .calc_time(ts)
                .map(|t| (t.as_secs_f64() * self.sample_rate as f64).round().max(0.0) as u64)
                .unwrap_or(0),
            None => ts.get().max(0) as u64,
        }
    }
}

impl StreamSource for DecoderStream {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn length_frames(&self) -> Option<u64> {
        self.length
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize, AudioError> {
        let ch = self.channels as usize;
        let want = out.len() / ch * ch;
        let mut written = 0;
        while written < want {
            if self.pending_pos >= self.pending.len() && (self.finished || !self.decode_next()?) {
                self.finished = true;
                break;
            }
            let n = (want - written).min(self.pending.len() - self.pending_pos);
            out[written..written + n]
                .copy_from_slice(&self.pending[self.pending_pos..self.pending_pos + n]);
            written += n;
            self.pending_pos += n;
        }
        Ok(written / ch)
    }

    fn seek(&mut self, frame: u64) -> Result<(), AudioError> {
        let secs = frame as f64 / self.sample_rate as f64;
        let time = Time::try_from_secs_f64(secs)
            .ok_or_else(|| AudioError::InvalidParameter(format!("cannot seek to {secs} s")))?;
        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| AudioError::Decode(e.to_string()))?;
        self.decoder.reset();
        self.pending.clear();
        self.pending_pos = 0;
        self.finished = false;
        let actual = self.ts_to_frames(seeked.actual_ts);
        self.skip_frames = frame.saturating_sub(actual);
        Ok(())
    }
}

/// Fold `channels`-wide interleaved samples to stereo, appending to `out`.
///
/// Layout assumption (WAV/Vorbis/FLAC order): 0 front-left, 1 front-right, 2 centre, 3 LFE,
/// 4 and 6 left surrounds, 5 and 7 right surrounds. Centre and surrounds enter at -3 dB, LFE is
/// dropped, and the sum is normalised by `1 / (1 + 0.707 * (1 + surround pairs))`.
fn fold_to_stereo(samples: &[f32], channels: usize, out: &mut Vec<f32>) {
    const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
    if channels == 0 {
        return;
    }
    let has_centre = channels >= 3;
    let surround_pairs = channels.saturating_sub(4) / 2;
    let norm = 1.0 / (1.0 + if has_centre { H } else { 0.0 } + H * surround_pairs as f32);
    for frame in samples.chunks_exact(channels) {
        let (mut l, mut r) = match channels {
            1 => (frame[0], frame[0]),
            _ => (frame[0], frame[1]),
        };
        if has_centre {
            l += H * frame[2];
            r += H * frame[2];
        }
        let mut c = 4;
        while c + 1 < channels {
            l += H * frame[c];
            r += H * frame[c + 1];
            c += 2;
        }
        out.push(l * norm);
        out.push(r * norm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_six_channels() {
        // FL=1, FR=0, C=1, LFE=1 (dropped), SL=0, SR=1.
        let mut out = Vec::new();
        fold_to_stereo(&[1.0, 0.0, 1.0, 1.0, 0.0, 1.0], 6, &mut out);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let norm = 1.0 / (1.0 + 2.0 * h);
        assert!((out[0] - (1.0 + h) * norm).abs() < 1e-6);
        assert!((out[1] - (h + h) * norm).abs() < 1e-6);
    }

    #[test]
    fn asset_rejects_ragged_samples() {
        assert!(SoundAsset::from_interleaved(vec![0.0; 3], 2, 48_000).is_err());
        assert!(SoundAsset::from_interleaved(vec![0.0; 4], 0, 48_000).is_err());
        assert!(SoundAsset::from_interleaved(vec![0.0; 4], 2, 0).is_err());
    }
}
