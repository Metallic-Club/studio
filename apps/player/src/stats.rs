//! What a run measured, and the `--bench` JSON.
//!
//! The numbers come from the engine's frame-stats collector ([`metallic_engine::perf`]), which the renderer the
//! Player installs ([`SceneRenderer`](metallic_engine::SceneRenderer)) fills every drawn frame: frame times with
//! p50 / p99 / 1% low, script, physics and render CPU time, GPU time per pass and the draws per pass. The Player
//! adds what only it knows: the scene, the preset, the frame counts and the wall time, and the graphics settings in
//! effect.

use std::fmt::Write as _;
use std::time::Duration;

use metallic_engine::perf::{GraphicsStatus, PerfSummary, json_string};

use crate::config::Preset;
use crate::scene::Scene;

/// What a run measured, as `--bench` prints it.
#[derive(Clone, Debug)]
pub struct Report {
    /// The scene that ran.
    pub scene: Scene,
    /// The graphics preset the run applied.
    pub preset: Option<Preset>,
    /// The graphics settings in effect at the end: the tier, Auto's calibration, the render scale and dynamic
    /// resolution.
    pub graphics: GraphicsStatus,
    /// Frames run before measuring.
    pub warmup_frames: u64,
    /// Every frame the engine ran, warm-up included.
    pub total_frames: u64,
    /// Wall time from the first frame to the end of the run.
    pub wall_time: Duration,
    /// Game time at the end, in seconds.
    pub game_time: f64,
    /// The drawn size in physical pixels, once the renderer has a window.
    pub surface: Option<(u32, u32)>,
    /// The measured frames (after the warm-up; the last [`PerfStats::window`] of them).
    ///
    /// [`PerfStats::window`]: metallic_engine::perf::PerfStats::window
    pub summary: PerfSummary,
}

impl Report {
    /// The report as one JSON object (the `--bench` output). Times are in milliseconds. The members after
    /// `surface` are [`PerfSummary::write_json_members`]'s, the same as the engine's `stress` example prints.
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(2048);
        let _ = write!(
            out,
            "{{\"scene\":{},\"preset\":{},\"warmup_frames\":{},\"total_frames\":{},\"wall_time_s\":{:.3},\
             \"game_time_s\":{:.3},",
            json_string(self.scene.name()),
            self.preset
                .map_or_else(|| "null".to_owned(), |preset| json_string(preset.name())),
            self.warmup_frames,
            self.total_frames,
            self.wall_time.as_secs_f64(),
            self.game_time
        );
        match self.surface {
            Some((width, height)) => {
                let _ = write!(out, "\"surface\":{{\"width\":{width},\"height\":{height}}},");
            }
            None => out.push_str("\"surface\":null,"),
        }
        out.push_str("\"graphics\":{");
        self.graphics.write_json_members(&mut out);
        out.push_str("},");
        self.summary.write_json_members(&mut out);
        out.push('}');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_is_one_json_object() {
        let mut summary = PerfSummary {
            frames: 2,
            ..PerfSummary::default()
        };
        summary.frame_ms.p50 = 16.0;
        summary.draws.opaque_draws = 4;
        let report = Report {
            scene: Scene::Demo,
            preset: Some(Preset::Low),
            graphics: GraphicsStatus {
                preset: Some(Preset::Low),
                ..GraphicsStatus::default()
            },
            warmup_frames: 2,
            total_frames: 4,
            wall_time: Duration::from_millis(1500),
            game_time: 1.25,
            surface: Some((1280, 720)),
            summary,
        };
        let json = report.to_json();
        for expected in [
            "{\"scene\":\"demo\",",
            "\"preset\":\"low\",",
            "\"total_frames\":4,",
            "\"wall_time_s\":1.500,",
            "\"surface\":{\"width\":1280,\"height\":720},",
            "\"graphics\":{\"preset\":\"low\",\"tier\":null,",
            "\"frames\":2,",
            "\"fps_p50\":62.5,",
            "\"opaque\":4,",
        ] {
            assert!(json.contains(expected), "missing {expected} in\n{json}");
        }
        assert!(json.ends_with("}}"), "{json}");
        assert_eq!(json.matches('{').count(), json.matches('}').count(), "{json}");

        let none = Report {
            preset: None,
            surface: None,
            ..report
        };
        let json = none.to_json();
        assert!(
            json.contains("\"preset\":null,") && json.contains("\"surface\":null,"),
            "{json}"
        );
    }
}
