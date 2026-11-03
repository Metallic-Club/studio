//! What a run measured: frame times, GPU times and what the renderer drew, and the `--bench` JSON.
//!
//! This is the Player's own small collector, built on what the engine exposes today ([`FrameStats`], the
//! renderer's draw statistics and GPU timings). #78 adds the engine's frame-stats collector and overlay; the Player
//! switches to it when that lands.
//!
//! [`FrameStats`]: metallic_engine::FrameStats

use std::fmt::Write as _;
use std::time::Duration;

use crate::config::Preset;
use crate::scene::Scene;

/// What the renderer drew in a frame: a copy of `metallic_render::DrawStats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Draws {
    /// Draws in the depth prepass.
    pub prepass: u32,
    /// Draws in the opaque pass.
    pub opaque: u32,
    /// Draws in the transparent pass.
    pub transparent: u32,
    /// Instances drawn in the opaque and transparent passes.
    pub instances: u32,
    /// Triangles drawn in the opaque and transparent passes.
    pub triangles: u64,
}

/// The samples a run collects, one per measured frame. Shared by the Player's event loop handler (frame times)
/// and its render hook (draws, GPU times).
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    /// Frames run before recording starts.
    pub warmup: u64,
    /// Wall time between the starts of consecutive frames, in milliseconds: what the frame rate is made of.
    pub frame_ms: Vec<f64>,
    /// Wall time spent inside `Engine::frame` (input, scripts, render sync, encode and present), in milliseconds.
    pub cpu_ms: Vec<f64>,
    /// The GPU time of each frame whose timestamps came back, in milliseconds (a frame or more late; none where the
    /// adapter has no timestamp queries).
    pub gpu_ms: Vec<f64>,
    /// The GPU time of each pass of the latest timed frame.
    pub gpu_passes: Vec<(&'static str, f64)>,
    /// What the last drawn frame drew.
    pub draws: Draws,
    /// The drawable size the renderer was last given, in physical pixels.
    pub surface: Option<(u32, u32)>,
    /// The renderer's frame index of the latest GPU timing recorded, so each is counted once.
    pub last_gpu_frame: Option<u64>,
}

impl Recorder {
    /// A recorder that ignores the first `warmup` frames.
    pub fn new(warmup: u64) -> Self {
        Self {
            warmup,
            ..Self::default()
        }
    }

    /// Whether engine frame number `frame` (from 1) is measured.
    pub fn measures(&self, frame: u64) -> bool {
        frame > self.warmup
    }
}

/// A distribution of per-frame values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    /// How many values.
    pub count: usize,
    /// The mean.
    pub mean: f64,
    /// The median.
    pub p50: f64,
    /// The 99th percentile (nearest rank).
    pub p99: f64,
    /// The largest value.
    pub max: f64,
}

impl Summary {
    /// The summary of `values`, or `None` when there are none.
    pub fn of(values: &[f64]) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        Some(Self {
            count: sorted.len(),
            mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
            p50: percentile(&sorted, 0.5),
            p99: percentile(&sorted, 0.99),
            max: sorted[sorted.len() - 1],
        })
    }
}

/// The nearest-rank percentile `p` (0 to 1) of `sorted`, which is not empty.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// The "1% low" frame rate: the frame rate of the slowest 1% of frames (at least one), from frame times in
/// milliseconds.
pub fn one_percent_low_fps(frame_ms: &[f64]) -> Option<f64> {
    if frame_ms.is_empty() {
        return None;
    }
    let mut sorted = frame_ms.to_vec();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let slowest = &sorted[..sorted.len().div_ceil(100)];
    let mean = slowest.iter().sum::<f64>() / slowest.len() as f64;
    (mean > 0.0).then(|| 1000.0 / mean)
}

/// What a run measured, as `--bench` prints it.
#[derive(Clone, Debug)]
pub struct Report {
    /// The scene that ran.
    pub scene: Scene,
    /// The preset asked for (not applied until #84).
    pub preset: Option<Preset>,
    /// Frames run before measuring.
    pub warmup_frames: u64,
    /// Every frame the engine ran, warm-up included.
    pub total_frames: u64,
    /// Wall time from the first frame to the end of the run.
    pub wall_time: Duration,
    /// Game time at the end, in seconds.
    pub game_time: f64,
    /// The measured samples.
    pub samples: Recorder,
}

impl Report {
    /// The report as one JSON object (the `--bench` output). Times are in milliseconds; a value that was not
    /// measured (GPU times on an adapter without timestamp queries) is `null`.
    pub fn to_json(&self) -> String {
        let s = &self.samples;
        let mut out = String::from("{\n");
        let mut field = |name: &str, value: String| {
            let _ = writeln!(out, "  \"{name}\": {value},");
        };
        field("scene", string(self.scene.name()));
        field("preset", self.preset.map_or_else(null, |preset| string(preset.name())));
        field("frames", s.cpu_ms.len().to_string());
        field("warmup_frames", self.warmup_frames.to_string());
        field("total_frames", self.total_frames.to_string());
        field("wall_time_s", number(self.wall_time.as_secs_f64()));
        field("game_time_s", number(self.game_time));
        field(
            "surface",
            s.surface.map_or_else(null, |(width, height)| {
                format!("{{ \"width\": {width}, \"height\": {height} }}")
            }),
        );
        field("frame_ms", summary(Summary::of(&s.frame_ms)));
        field("cpu_ms", summary(Summary::of(&s.cpu_ms)));
        field("gpu_ms", summary(Summary::of(&s.gpu_ms)));
        let fps = Summary::of(&s.frame_ms).map(|frames| 1000.0 / frames.mean);
        field(
            "fps",
            format!(
                "{{ \"mean\": {}, \"low_1pct\": {} }}",
                optional(fps),
                optional(one_percent_low_fps(&s.frame_ms))
            ),
        );
        let passes: Vec<String> = s
            .gpu_passes
            .iter()
            .map(|(pass, ms)| format!("{}: {}", string(pass), number(*ms)))
            .collect();
        field("gpu_passes_ms", format!("{{ {} }}", passes.join(", ")));
        let d = s.draws;
        let _ = write!(
            out,
            "  \"draws\": {{ \"prepass\": {}, \"opaque\": {}, \"transparent\": {}, \"instances\": {}, \"triangles\": {} }}\n}}",
            d.prepass, d.opaque, d.transparent, d.instances, d.triangles
        );
        out
    }
}

fn null() -> String {
    "null".to_owned()
}

/// A JSON string of `text`, which holds no characters JSON must escape (names chosen in this crate).
fn string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A finite number with three decimals, or `null`.
fn number(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.3}")
    } else {
        null()
    }
}

fn optional(value: Option<f64>) -> String {
    value.map_or_else(null, number)
}

fn summary(summary: Option<Summary>) -> String {
    summary.map_or_else(null, |s| {
        format!(
            "{{ \"mean\": {}, \"p50\": {}, \"p99\": {}, \"max\": {} }}",
            number(s.mean),
            number(s.p50),
            number(s.p99),
            number(s.max)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_use_nearest_rank_percentiles() {
        let values: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = Summary::of(&values).unwrap();
        assert_eq!(s.count, 100);
        assert_eq!(s.mean, 50.5);
        assert_eq!(s.p50, 50.0);
        assert_eq!(s.p99, 99.0);
        assert_eq!(s.max, 100.0);
        assert_eq!(Summary::of(&[7.0]).unwrap().p99, 7.0);
        assert!(Summary::of(&[]).is_none());
    }

    #[test]
    fn the_one_percent_low_is_the_rate_of_the_slowest_frames() {
        // 199 frames of 10 ms and one of 40 ms: the slowest 1% is two frames, averaging 25 ms.
        let mut frames = vec![10.0; 199];
        frames.push(40.0);
        assert_eq!(one_percent_low_fps(&frames), Some(40.0));
        assert_eq!(one_percent_low_fps(&[20.0]), Some(50.0));
        assert_eq!(one_percent_low_fps(&[]), None);
    }

    #[test]
    fn the_report_is_json_with_nulls_for_what_was_not_measured() {
        let report = Report {
            scene: Scene::Demo,
            preset: Some(Preset::Low),
            warmup_frames: 2,
            total_frames: 4,
            wall_time: Duration::from_millis(1500),
            game_time: 1.25,
            samples: Recorder {
                frame_ms: vec![16.0, 18.0],
                cpu_ms: vec![2.0, 3.0],
                draws: Draws {
                    opaque: 4,
                    instances: 90,
                    triangles: 12_000,
                    ..Draws::default()
                },
                surface: Some((1280, 720)),
                gpu_passes: vec![("opaque", 0.5)],
                ..Recorder::new(2)
            },
        };
        let json = report.to_json();
        for expected in [
            "\"scene\": \"demo\",",
            "\"preset\": \"low\",",
            "\"frames\": 2,",
            "\"total_frames\": 4,",
            "\"wall_time_s\": 1.500,",
            "\"surface\": { \"width\": 1280, \"height\": 720 },",
            "\"frame_ms\": { \"mean\": 17.000, \"p50\": 16.000, \"p99\": 18.000, \"max\": 18.000 },",
            "\"gpu_ms\": null,",
            "\"fps\": { \"mean\": 58.824, \"low_1pct\": 55.556 },",
            "\"gpu_passes_ms\": { \"opaque\": 0.500 },",
            "\"draws\": { \"prepass\": 0, \"opaque\": 4, \"transparent\": 0, \"instances\": 90, \"triangles\": 12000 }",
        ] {
            assert!(json.contains(expected), "missing {expected} in\n{json}");
        }
        assert!(json.starts_with("{\n") && json.ends_with("\n}"));
    }
}
