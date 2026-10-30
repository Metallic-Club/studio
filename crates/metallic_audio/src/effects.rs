//! Group effects (Roblox `SoundEffect` subclasses) and the filters they are built from.
//!
//! Parameters use Roblox property names in snake case; levels are in decibels like Roblox.
//! Processors allocate only when they are created (on the game thread); changing parameters on a
//! live effect never allocates, so it is safe on the audio thread.
//!
//! Filters follow the well-known RBJ "Audio EQ Cookbook" biquad formulas; the reverb is a
//! Schroeder/Moorer design (parallel damped combs into series all-passes). Both are textbook
//! public-domain techniques, written fresh here.

use std::f32::consts::TAU;

/// An effect and its parameters. Insert on a group with
/// [`AudioEngine::add_effect`](crate::AudioEngine::add_effect).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effect {
    /// `ReverbSoundEffect`: room reflections and a decaying tail.
    Reverb(ReverbParams),
    /// `EqualizerSoundEffect`: three-band gain.
    Equalizer(EqualizerParams),
    /// `CompressorSoundEffect`: evens out loudness above a threshold.
    Compressor(CompressorParams),
    /// `DistortionSoundEffect`: soft clipping.
    Distortion(DistortionParams),
    /// `EchoSoundEffect`: a feedback delay.
    Echo(EchoParams),
    /// Second-order low-pass (not a Roblox class; for muffling a whole group, e.g. underwater).
    LowPass(FilterParams),
    /// Second-order high-pass (not a Roblox class; for thin radio-like groups).
    HighPass(FilterParams),
}

/// `ReverbSoundEffect` parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReverbParams {
    /// `DecayTime`: seconds for the tail to fall by 60 dB (0.1..=20). Default 1.5.
    pub decay_time: f32,
    /// `Density` (0..=1): how much high frequencies survive each reflection; 1 = bright, dense
    /// tail, 0 = dark tail. Default 1. (Mapping is the assistant's call.)
    pub density: f32,
    /// `Diffusion` (0..=1): smears the reflections; 0 = audible discrete echoes. Default 1.
    pub diffusion: f32,
    /// `DryLevel` in dB (-80..=20). Default -6.
    pub dry_level: f32,
    /// `WetLevel` in dB (-80..=20). Default 0.
    pub wet_level: f32,
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self {
            decay_time: 1.5,
            density: 1.0,
            diffusion: 1.0,
            dry_level: -6.0,
            wet_level: 0.0,
        }
    }
}

/// `EqualizerSoundEffect` parameters. Gains are applied within -60..=20 dB (-60 dB is already
/// inaudible next to the other bands).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqualizerParams {
    /// `LowGain` in dB below `mid_range.0` (-80..=10). Default 0.
    pub low_gain: f32,
    /// `MidGain` in dB between the two crossover frequencies (-80..=10). Default 0.
    pub mid_gain: f32,
    /// `HighGain` in dB above `mid_range.1` (-80..=10). Default 0.
    pub high_gain: f32,
    /// `MidRange` crossover frequencies in Hz `(low, high)`. Default (400, 4000).
    pub mid_range: (f32, f32),
}

impl Default for EqualizerParams {
    fn default() -> Self {
        Self {
            low_gain: 0.0,
            mid_gain: 0.0,
            high_gain: 0.0,
            mid_range: (400.0, 4_000.0),
        }
    }
}

/// `CompressorSoundEffect` parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressorParams {
    /// `Threshold` in dB above which gain is reduced (-60..=0). Default -40.
    pub threshold: f32,
    /// `Ratio` (1..=50): 4 means 4 dB over the threshold come out as 1 dB. Default 40.
    pub ratio: f32,
    /// `Attack` in seconds (0.001..=1). Default 0.1.
    pub attack: f32,
    /// `Release` in seconds (0.001..=5). Default 0.1.
    pub release: f32,
    /// `GainMakeup` in dB added after compression (0..=30). Default 0.
    pub gain_makeup: f32,
}

impl Default for CompressorParams {
    fn default() -> Self {
        Self {
            threshold: -40.0,
            ratio: 40.0,
            attack: 0.1,
            release: 0.1,
            gain_makeup: 0.0,
        }
    }
}

/// `DistortionSoundEffect` parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistortionParams {
    /// `Level` (0..=1): drive into a soft clipper. Default 0.75.
    pub level: f32,
}

impl Default for DistortionParams {
    fn default() -> Self {
        Self { level: 0.75 }
    }
}

/// `EchoSoundEffect` parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EchoParams {
    /// `Delay` in seconds between repeats (0.01..=2). Default 1.
    pub delay: f32,
    /// `Feedback` (0..=0.95): how much of each repeat feeds the next. Default 0.5.
    pub feedback: f32,
    /// `DryLevel` in dB. Default 0.
    pub dry_level: f32,
    /// `WetLevel` in dB. Default 0.
    pub wet_level: f32,
}

impl Default for EchoParams {
    fn default() -> Self {
        Self {
            delay: 1.0,
            feedback: 0.5,
            dry_level: 0.0,
            wet_level: 0.0,
        }
    }
}

/// Low-pass or high-pass filter parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterParams {
    /// Cutoff frequency in Hz.
    pub cutoff_hz: f32,
    /// Resonance; 0.707 is a flat Butterworth response.
    pub q: f32,
}

impl Default for FilterParams {
    fn default() -> Self {
        Self {
            cutoff_hz: 2_000.0,
            q: std::f32::consts::FRAC_1_SQRT_2,
        }
    }
}

/// Decibels to a linear amplitude factor; -80 dB and below count as silence.
pub fn db_to_gain(db: f32) -> f32 {
    if !db.is_finite() || db <= -80.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Longest echo delay in seconds (sets the echo buffer size).
pub const MAX_ECHO_DELAY: f32 = 2.0;

// ---------------------------------------------------------------------------------------------
// Filters

/// Second-order IIR section in transposed direct form II, with state for two channels.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: [f32; 2],
    z2: [f32; 2],
}

impl Biquad {
    fn identity() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: [0.0; 2],
            z2: [0.0; 2],
        }
    }

    pub(crate) fn low_pass(sample_rate: f32, cutoff: f32, q: f32) -> Self {
        let mut f = Self::identity();
        f.set_low_pass(sample_rate, cutoff, q);
        f
    }

    pub(crate) fn high_pass(sample_rate: f32, cutoff: f32, q: f32) -> Self {
        let mut f = Self::identity();
        f.set_high_pass(sample_rate, cutoff, q);
        f
    }

    fn omega(sample_rate: f32, cutoff: f32, q: f32) -> (f32, f32) {
        let nyquist_safe = sample_rate * 0.49;
        let f0 = if cutoff.is_finite() {
            cutoff.clamp(10.0, nyquist_safe)
        } else {
            nyquist_safe
        };
        let q = if q.is_finite() {
            q.max(0.05)
        } else {
            std::f32::consts::FRAC_1_SQRT_2
        };
        let w0 = TAU * f0 / sample_rate;
        (w0.cos(), w0.sin() / (2.0 * q))
    }

    pub(crate) fn set_low_pass(&mut self, sample_rate: f32, cutoff: f32, q: f32) {
        let (cos, alpha) = Self::omega(sample_rate, cutoff, q);
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 - cos) * 0.5 / a0;
        self.b1 = (1.0 - cos) / a0;
        self.b2 = self.b0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    pub(crate) fn set_high_pass(&mut self, sample_rate: f32, cutoff: f32, q: f32) {
        let (cos, alpha) = Self::omega(sample_rate, cutoff, q);
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 + cos) * 0.5 / a0;
        self.b1 = -(1.0 + cos) / a0;
        self.b2 = self.b0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    /// Shelving filter (slope 1): `gain_db` below `corner` (low shelf) or above it (high shelf).
    pub(crate) fn set_shelf(&mut self, sample_rate: f32, corner: f32, gain_db: f32, high: bool) {
        let (cos, _) = Self::omega(sample_rate, corner, std::f32::consts::FRAC_1_SQRT_2);
        let a = 10f32.powf(gain_db / 40.0);
        let sin = (1.0 - cos * cos).max(0.0).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * sin * 0.5 * std::f32::consts::SQRT_2;
        let (ap1, am1) = (a + 1.0, a - 1.0);
        let (b0, b1, b2, a0, a1, a2) = if high {
            (
                a * (ap1 + am1 * cos + two_sqrt_a_alpha),
                -2.0 * a * (am1 + ap1 * cos),
                a * (ap1 + am1 * cos - two_sqrt_a_alpha),
                ap1 - am1 * cos + two_sqrt_a_alpha,
                2.0 * (am1 - ap1 * cos),
                ap1 - am1 * cos - two_sqrt_a_alpha,
            )
        } else {
            (
                a * (ap1 - am1 * cos + two_sqrt_a_alpha),
                2.0 * a * (am1 - ap1 * cos),
                a * (ap1 - am1 * cos - two_sqrt_a_alpha),
                ap1 + am1 * cos + two_sqrt_a_alpha,
                -2.0 * (am1 + ap1 * cos),
                ap1 + am1 * cos - two_sqrt_a_alpha,
            )
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    #[inline]
    pub(crate) fn tick(&mut self, ch: usize, x: f32) -> f32 {
        let y = self.b0 * x + self.z1[ch];
        self.z1[ch] = self.b1 * x - self.a1 * y + self.z2[ch];
        self.z2[ch] = self.b2 * x - self.a2 * y;
        y
    }

    fn process(&mut self, buf: &mut [[f32; 2]]) {
        for frame in buf.iter_mut() {
            frame[0] = self.tick(0, frame[0]);
            frame[1] = self.tick(1, frame[1]);
        }
    }
}

/// One-pole low-pass smoother; `a` 1 passes everything.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OnePole {
    pub(crate) state: [f32; 2],
}

impl OnePole {
    /// Coefficient for a cutoff; >= 19 kHz (or near Nyquist) returns 1 (bypass).
    pub(crate) fn coefficient(sample_rate: f32, cutoff_hz: f32) -> f32 {
        if !cutoff_hz.is_finite() || cutoff_hz >= 19_000.0 || cutoff_hz >= sample_rate * 0.45 {
            return 1.0;
        }
        1.0 - (-TAU * cutoff_hz.max(10.0) / sample_rate).exp()
    }

    #[inline]
    pub(crate) fn tick(&mut self, ch: usize, a: f32, x: f32) -> f32 {
        self.state[ch] += a * (x - self.state[ch]);
        self.state[ch]
    }
}

// ---------------------------------------------------------------------------------------------
// Processors

/// Live processor of one effect.
pub(crate) enum EffectState {
    Reverb(Box<Reverb>),
    Equalizer(Equalizer),
    Compressor(Compressor),
    Distortion(f32),
    Echo(Box<Echo>),
    Filter(Biquad, bool),
}

impl EffectState {
    /// Build the processor (allocates delay lines; call off the audio thread).
    pub(crate) fn new(effect: &Effect, sample_rate: u32) -> Self {
        let fs = sample_rate as f32;
        match effect {
            Effect::Reverb(p) => EffectState::Reverb(Box::new(Reverb::new(p, fs))),
            Effect::Equalizer(p) => EffectState::Equalizer(Equalizer::new(p, fs)),
            Effect::Compressor(p) => EffectState::Compressor(Compressor::new(p, fs)),
            Effect::Distortion(p) => EffectState::Distortion(p.level),
            Effect::Echo(p) => EffectState::Echo(Box::new(Echo::new(p, fs))),
            Effect::LowPass(p) => {
                EffectState::Filter(Biquad::low_pass(fs, p.cutoff_hz, p.q), false)
            }
            Effect::HighPass(p) => {
                EffectState::Filter(Biquad::high_pass(fs, p.cutoff_hz, p.q), true)
            }
        }
    }

    /// Change parameters in place without allocating. Returns false if `effect` is a different
    /// kind than this processor (the caller then rebuilds it).
    pub(crate) fn set(&mut self, effect: &Effect, sample_rate: u32) -> bool {
        let fs = sample_rate as f32;
        match (self, effect) {
            (EffectState::Reverb(r), Effect::Reverb(p)) => r.set(p, fs),
            (EffectState::Equalizer(e), Effect::Equalizer(p)) => e.set(p, fs),
            (EffectState::Compressor(c), Effect::Compressor(p)) => c.set(p, fs),
            (EffectState::Distortion(level), Effect::Distortion(p)) => *level = p.level,
            (EffectState::Echo(e), Effect::Echo(p)) => e.set(p, fs),
            (EffectState::Filter(f, false), Effect::LowPass(p)) => {
                f.set_low_pass(fs, p.cutoff_hz, p.q)
            }
            (EffectState::Filter(f, true), Effect::HighPass(p)) => {
                f.set_high_pass(fs, p.cutoff_hz, p.q)
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn process(&mut self, buf: &mut [[f32; 2]]) {
        match self {
            EffectState::Reverb(r) => r.process(buf),
            EffectState::Equalizer(e) => e.process(buf),
            EffectState::Compressor(c) => c.process(buf),
            EffectState::Distortion(level) => distort(*level, buf),
            EffectState::Echo(e) => e.process(buf),
            EffectState::Filter(f, _) => f.process(buf),
        }
    }
}

fn distort(level: f32, buf: &mut [[f32; 2]]) {
    let level = if level.is_finite() {
        level.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if level <= 0.0 {
        return;
    }
    // tanh(d x) / tanh(d): full scale stays full scale, quieter signals are driven harder.
    let drive = 1.0 + 9.0 * level;
    let norm = 1.0 / drive.tanh();
    for frame in buf.iter_mut() {
        frame[0] = (drive * frame[0]).tanh() * norm;
        frame[1] = (drive * frame[1]).tanh() * norm;
    }
}

/// Three-band equaliser: `mid_gain` overall, a low shelf at `mid_range.0` worth
/// `low_gain - mid_gain` and a high shelf at `mid_range.1` worth `high_gain - mid_gain`. Shelving
/// filters keep the bands phase-coherent, so a cut band really is cut; with all gains at 0 dB the
/// filter is exactly transparent. Gains are limited to -60..=20 dB.
pub(crate) struct Equalizer {
    low: Biquad,
    high: Biquad,
    gain: f32,
}

impl Equalizer {
    fn new(p: &EqualizerParams, fs: f32) -> Self {
        let mut e = Self {
            low: Biquad::identity(),
            high: Biquad::identity(),
            gain: 1.0,
        };
        e.set(p, fs);
        e
    }

    fn set(&mut self, p: &EqualizerParams, fs: f32) {
        let clamp = |db: f32| {
            if db.is_finite() {
                db.clamp(-60.0, 20.0)
            } else {
                0.0
            }
        };
        let (low, mid, high) = (clamp(p.low_gain), clamp(p.mid_gain), clamp(p.high_gain));
        let lo = p.mid_range.0.min(p.mid_range.1);
        let hi = p.mid_range.0.max(p.mid_range.1);
        self.low.set_shelf(fs, lo, low - mid, false);
        self.high.set_shelf(fs, hi, high - mid, true);
        self.gain = 10f32.powf(mid / 20.0);
    }

    fn process(&mut self, buf: &mut [[f32; 2]]) {
        for frame in buf.iter_mut() {
            for (ch, x) in frame.iter_mut().enumerate() {
                let y = self.low.tick(ch, *x);
                *x = self.gain * self.high.tick(ch, y);
            }
        }
    }
}

/// Feed-forward peak compressor, stereo-linked.
pub(crate) struct Compressor {
    attack: f32,
    release: f32,
    threshold_db: f32,
    slope: f32,
    makeup: f32,
    envelope: f32,
}

impl Compressor {
    fn new(p: &CompressorParams, fs: f32) -> Self {
        let mut c = Self {
            attack: 0.0,
            release: 0.0,
            threshold_db: 0.0,
            slope: 0.0,
            makeup: 1.0,
            envelope: 0.0,
        };
        c.set(p, fs);
        c
    }

    fn set(&mut self, p: &CompressorParams, fs: f32) {
        let coef = |secs: f32| {
            let secs = if secs.is_finite() {
                secs.clamp(1e-4, 10.0)
            } else {
                0.1
            };
            (-1.0 / (secs * fs)).exp()
        };
        self.attack = coef(p.attack);
        self.release = coef(p.release);
        self.threshold_db = if p.threshold.is_finite() {
            p.threshold.clamp(-80.0, 0.0)
        } else {
            0.0
        };
        let ratio = if p.ratio.is_finite() {
            p.ratio.clamp(1.0, 100.0)
        } else {
            1.0
        };
        self.slope = 1.0 - 1.0 / ratio;
        self.makeup = db_to_gain(p.gain_makeup.clamp(-80.0, 40.0));
    }

    fn process(&mut self, buf: &mut [[f32; 2]]) {
        for frame in buf.iter_mut() {
            let peak = frame[0].abs().max(frame[1].abs());
            let coef = if peak > self.envelope {
                self.attack
            } else {
                self.release
            };
            self.envelope = coef * self.envelope + (1.0 - coef) * peak;
            let gain = if self.envelope > 1e-6 {
                let level_db = 20.0 * self.envelope.log10();
                let over = level_db - self.threshold_db;
                if over > 0.0 {
                    db_to_gain(-over * self.slope)
                } else {
                    1.0
                }
            } else {
                1.0
            };
            let g = gain * self.makeup;
            frame[0] *= g;
            frame[1] *= g;
        }
    }
}

/// Feedback delay per channel.
pub(crate) struct Echo {
    buffer: Vec<[f32; 2]>,
    write: usize,
    delay: usize,
    feedback: f32,
    dry: f32,
    wet: f32,
}

impl Echo {
    fn new(p: &EchoParams, fs: f32) -> Self {
        let len = (MAX_ECHO_DELAY * fs) as usize + 2;
        let mut e = Self {
            buffer: vec![[0.0; 2]; len],
            write: 0,
            delay: 1,
            feedback: 0.0,
            dry: 1.0,
            wet: 1.0,
        };
        e.set(p, fs);
        e
    }

    fn set(&mut self, p: &EchoParams, fs: f32) {
        let delay = if p.delay.is_finite() {
            p.delay.clamp(0.001, MAX_ECHO_DELAY)
        } else {
            1.0
        };
        self.delay = ((delay * fs) as usize).clamp(1, self.buffer.len() - 1);
        self.feedback = if p.feedback.is_finite() {
            p.feedback.clamp(0.0, 0.95)
        } else {
            0.0
        };
        self.dry = db_to_gain(p.dry_level);
        self.wet = db_to_gain(p.wet_level);
    }

    fn process(&mut self, buf: &mut [[f32; 2]]) {
        let len = self.buffer.len();
        for frame in buf.iter_mut() {
            let read = (self.write + len - self.delay) % len;
            let delayed = self.buffer[read];
            for (ch, sample) in frame.iter_mut().enumerate() {
                self.buffer[self.write][ch] = *sample + self.feedback * delayed[ch];
                *sample = self.dry * *sample + self.wet * delayed[ch];
            }
            self.write = (self.write + 1) % len;
        }
    }
}

/// Comb delay lengths in milliseconds (mutually non-harmonic, 25-37 ms).
const COMB_MS: [f32; 8] = [25.31, 26.93, 28.97, 30.75, 32.24, 33.81, 35.31, 36.67];
/// All-pass delay lengths in milliseconds.
const ALLPASS_MS: [f32; 4] = [12.61, 10.0, 7.73, 5.1];
/// Extra delay of the right channel for stereo width.
const STEREO_SPREAD_MS: f32 = 0.53;

struct Comb {
    buffer: Vec<f32>,
    index: usize,
    feedback: f32,
    /// Output scale that keeps the comb's noise gain near 1 whatever the decay time.
    norm: f32,
    store: f32,
}

impl Comb {
    #[inline]
    fn tick(&mut self, x: f32, damp: f32) -> f32 {
        let out = self.buffer[self.index];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buffer[self.index] = x + self.store * self.feedback;
        self.index += 1;
        if self.index == self.buffer.len() {
            self.index = 0;
        }
        out * self.norm
    }
}

struct AllPass {
    buffer: Vec<f32>,
    index: usize,
}

impl AllPass {
    #[inline]
    fn tick(&mut self, x: f32, g: f32) -> f32 {
        // v[n] = x[n] + g v[n-D];  y[n] = v[n-D] - g v[n]
        let delayed = self.buffer[self.index];
        let v = x + g * delayed;
        self.buffer[self.index] = v;
        self.index += 1;
        if self.index == self.buffer.len() {
            self.index = 0;
        }
        delayed - g * v
    }
}

/// Schroeder/Moorer reverb: 8 damped combs in parallel, then 4 all-passes, per channel.
pub(crate) struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<AllPass>; 2],
    damp: f32,
    allpass_gain: f32,
    dry: f32,
    wet: f32,
}

impl Reverb {
    fn new(p: &ReverbParams, fs: f32) -> Self {
        let samples = |ms: f32, ch: usize| {
            ((ms + ch as f32 * STEREO_SPREAD_MS) * 0.001 * fs).max(1.0) as usize
        };
        let combs = [0, 1].map(|ch| {
            COMB_MS
                .iter()
                .map(|&ms| Comb {
                    buffer: vec![0.0; samples(ms, ch)],
                    index: 0,
                    feedback: 0.0,
                    norm: 1.0,
                    store: 0.0,
                })
                .collect()
        });
        let allpasses = [0, 1].map(|ch| {
            ALLPASS_MS
                .iter()
                .map(|&ms| AllPass {
                    buffer: vec![0.0; samples(ms, ch)],
                    index: 0,
                })
                .collect()
        });
        let mut r = Self {
            combs,
            allpasses,
            damp: 0.0,
            allpass_gain: 0.5,
            dry: 1.0,
            wet: 1.0,
        };
        r.set(p, fs);
        r
    }

    fn set(&mut self, p: &ReverbParams, fs: f32) {
        let decay = if p.decay_time.is_finite() {
            p.decay_time.clamp(0.1, 20.0)
        } else {
            1.5
        };
        let density = if p.density.is_finite() {
            p.density.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let diffusion = if p.diffusion.is_finite() {
            p.diffusion.clamp(0.0, 1.0)
        } else {
            1.0
        };
        for comb in self.combs.iter_mut().flatten() {
            // RT60: the loop gain per pass is 10^(-3 * delay / T60).
            let delay_secs = comb.buffer.len() as f32 / fs;
            comb.feedback = 10f32.powf(-3.0 * delay_secs / decay).min(0.995);
            comb.norm = (1.0 - comb.feedback * comb.feedback).sqrt();
        }
        self.damp = 0.05 + 0.5 * (1.0 - density);
        self.allpass_gain = 0.25 + 0.45 * diffusion;
        self.dry = db_to_gain(p.dry_level);
        // Eight uncorrelated combs at unit noise gain sum to sqrt(8).
        self.wet = db_to_gain(p.wet_level) / (COMB_MS.len() as f32).sqrt();
    }

    fn process(&mut self, buf: &mut [[f32; 2]]) {
        for frame in buf.iter_mut() {
            let input = 0.5 * (frame[0] + frame[1]);
            for (ch, sample) in frame.iter_mut().enumerate() {
                let mut acc = 0.0;
                for comb in self.combs[ch].iter_mut() {
                    acc += comb.tick(input, self.damp);
                }
                for ap in self.allpasses[ch].iter_mut() {
                    acc = ap.tick(acc, self.allpass_gain);
                }
                *sample = self.dry * *sample + self.wet * acc;
            }
        }
    }
}

/// Magnitude response of a biquad at `freq` (for tests and tooling).
#[cfg(test)]
pub(crate) fn biquad_magnitude(f: &Biquad, sample_rate: f32, freq: f32) -> f32 {
    let w = TAU * freq / sample_rate;
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let num_re = f.b0 + f.b1 * c1 + f.b2 * c2;
    let num_im = -(f.b1 * s1 + f.b2 * s2);
    let den_re = 1.0 + f.a1 * c1 + f.a2 * c2;
    let den_im = -(f.a1 * s1 + f.a2 * s2);
    ((num_re * num_re + num_im * num_im) / (den_re * den_re + den_im * den_im)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_pass_magnitude_at_cutoff_is_minus_3_db() {
        let f = Biquad::low_pass(48_000.0, 1_000.0, std::f32::consts::FRAC_1_SQRT_2);
        let m = biquad_magnitude(&f, 48_000.0, 1_000.0);
        assert!((m - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3, "{m}");
        assert!((biquad_magnitude(&f, 48_000.0, 10.0) - 1.0).abs() < 1e-3);
        assert!(biquad_magnitude(&f, 48_000.0, 10_000.0) < 0.02);
    }

    #[test]
    fn high_pass_magnitude() {
        let f = Biquad::high_pass(48_000.0, 1_000.0, std::f32::consts::FRAC_1_SQRT_2);
        assert!(
            (biquad_magnitude(&f, 48_000.0, 1_000.0) - std::f32::consts::FRAC_1_SQRT_2).abs()
                < 1e-3
        );
        assert!((biquad_magnitude(&f, 48_000.0, 15_000.0) - 1.0).abs() < 0.01);
        assert!(biquad_magnitude(&f, 48_000.0, 50.0) < 0.01);
    }

    #[test]
    fn shelves_reach_their_gain() {
        let fs = 48_000.0;
        let mut low = Biquad::identity();
        low.set_shelf(fs, 500.0, -24.0, false);
        let db = |f: &Biquad, hz: f32| 20.0 * biquad_magnitude(f, fs, hz).log10();
        assert!((db(&low, 20.0) + 24.0).abs() < 0.5, "{}", db(&low, 20.0));
        assert!(db(&low, 15_000.0).abs() < 0.1);
        assert!(
            (db(&low, 500.0) + 12.0).abs() < 0.5,
            "half gain at the corner"
        );
        let mut high = Biquad::identity();
        high.set_shelf(fs, 4_000.0, 12.0, true);
        assert!((db(&high, 20_000.0) - 12.0).abs() < 0.5);
        assert!(db(&high, 50.0).abs() < 0.1);
        let mut flat = Biquad::identity();
        flat.set_shelf(fs, 1_000.0, 0.0, true);
        assert!((biquad_magnitude(&flat, fs, 3_000.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn db_conversion() {
        assert!((db_to_gain(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_gain(-6.0206) - 0.5).abs() < 1e-4);
        assert_eq!(db_to_gain(-80.0), 0.0);
        assert_eq!(db_to_gain(f32::NAN), 0.0);
    }

    #[test]
    fn set_rejects_other_kind() {
        let mut s = EffectState::new(&Effect::Distortion(DistortionParams::default()), 48_000);
        assert!(!s.set(&Effect::Echo(EchoParams::default()), 48_000));
        assert!(s.set(&Effect::Distortion(DistortionParams { level: 0.1 }), 48_000));
        let mut lp = EffectState::new(&Effect::LowPass(FilterParams::default()), 48_000);
        assert!(!lp.set(&Effect::HighPass(FilterParams::default()), 48_000));
    }

    #[test]
    fn reverb_comb_feedback_matches_rt60() {
        let fs = 48_000.0;
        let r = Reverb::new(
            &ReverbParams {
                decay_time: 2.0,
                ..Default::default()
            },
            fs,
        );
        for comb in r.combs.iter().flatten() {
            let delay = comb.buffer.len() as f32 / fs;
            // After T60 seconds the comb loop has decayed by 60 dB.
            let passes = 2.0 / delay;
            let db = 20.0 * comb.feedback.powf(passes).log10();
            assert!((db + 60.0).abs() < 0.1, "{db}");
        }
    }
}
