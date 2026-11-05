//! Frame statistics (ADR 0008 §10): what each frame cost, summarised as percentiles, and the performance overlay
//! that shows them.
//!
//! [`PerfStats`] keeps the last `window` frames in fixed rings: the frame time (wall time from one drawn frame to
//! the next), the script, physics and render CPU time, the per-pass GPU time from timestamps (which arrive one to
//! three frames late, so they are kept apart and counted once per timed frame), and the last frame's draws.
//! [`PerfStats::summary`] gives the mean, median (p50), p99 and maximum of each, and the **1% low**: the mean frame
//! time of the slowest 1% of frames (at least one), and the frame rate that is. Nothing allocates after
//! [`PerfStats::new`]: the rings and the scratch buffer the percentiles are sorted in are sized up front.
//!
//! [`draw_overlay`] writes a summary and a frame-time graph into the renderer's [`Overlay`]; [`SceneRenderer`]
//! does this every frame while its overlay is on ([`SceneRenderer::set_overlay`]).
//!
//! [`SceneRenderer`]: crate::SceneRenderer
//! [`SceneRenderer::set_overlay`]: crate::SceneRenderer::set_overlay

use std::time::Duration;

use metallic_render::resolution::ResolutionStats;
use metallic_render::settings::{Preset, Tier};
use metallic_render::{DrawStats, FrameTimings, MAX_CASCADES, Overlay, PassId, PipelineStats, PostStats};

/// The graphics settings in effect, as the overlay and the reports show them
/// ([`SceneRenderer::graphics_status`](crate::SceneRenderer::graphics_status)).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GraphicsStatus {
    /// The player's preset; `None` when no graphics settings were set (the per-system settings as set one by one).
    pub preset: Option<Preset>,
    /// The tier drawn at (for Auto, the calibrated one, or the one being measured).
    pub tier: Option<Tier>,
    /// The tier Auto's calibration is measuring, while it runs.
    pub calibrating: Option<Tier>,
    /// The render scale and dynamic resolution of the last frame.
    pub resolution: ResolutionStats,
    /// The post chain: the scene colour's format, the anti-aliasing, bloom, and the frame's passes.
    pub post: PostStats,
    /// The renderer's pipeline compiles: how many, how long, and how many are still pending.
    pub pipelines: PipelineStats,
}

impl GraphicsStatus {
    /// The status as the members of a JSON object, without the braces: `preset`, `tier` and `calibrating` (names,
    /// or `null`), `resolution` (`scale`, `width`, `height`, `dynamic`, `target_ms`, `min_scale`, `max_scale`),
    /// `post` (`scene_format`, `finish`, `anti_aliasing`, `bloom_levels`, `render_passes`, `full_screen_passes`, and
    /// `passes`: the planned passes' names in order, bloom's chain as one) and `pipelines` (`compiled`, `pending`,
    /// `failed`, `workers`, `first_frame_ms`, `all_ms` (`null` while some are pending), `compile_ms` (CPU time summed
    /// over the workers), `switch_waiting` and `cache_loaded` (bytes, `null` without a pipeline cache)).
    pub fn write_json_members(&self, out: &mut String) {
        use std::fmt::Write as _;
        let name = |name: Option<&str>| name.map_or_else(|| "null".to_owned(), json_string);
        let r = &self.resolution;
        let _ = write!(
            out,
            "\"preset\":{},\"tier\":{},\"calibrating\":{},\"resolution\":{{\"scale\":{:.4},\"width\":{},\
             \"height\":{},\"dynamic\":{},\"target_ms\":{:.3},\"min_scale\":{:.3},\"max_scale\":{:.3}}}",
            name(self.preset.map(Preset::name)),
            name(self.tier.map(Tier::name)),
            name(self.calibrating.map(Tier::name)),
            r.scale,
            r.width,
            r.height,
            r.dynamic,
            r.target_ms,
            r.min_scale,
            r.max_scale
        );
        let p = &self.post;
        let _ = write!(
            out,
            ",\"post\":{{\"scene_format\":{},\"finish\":{},\"anti_aliasing\":{},\"bloom_levels\":{},\
             \"render_passes\":{},\"full_screen_passes\":{},\"passes\":[",
            json_string(p.format_name()),
            p.finish,
            json_string(p.anti_aliasing().name()),
            p.bloom_levels,
            p.render_passes,
            p.full_screen_passes
        );
        for (i, pass) in p.passes().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&json_string(pass.name()));
        }
        out.push_str("]}");
        let c = &self.pipelines;
        let option = |value: Option<String>| value.unwrap_or_else(|| "null".to_owned());
        let _ = write!(
            out,
            ",\"pipelines\":{{\"compiled\":{},\"pending\":{},\"failed\":{},\"workers\":{},\"first_frame_ms\":{:.1},\
             \"all_ms\":{},\"compile_ms\":{:.1},\"switch_waiting\":{},\"cache_loaded\":{}}}",
            c.compiled,
            c.pending,
            c.failed,
            c.workers,
            c.first_frame_ms,
            option(c.all_ms.map(|ms| format!("{ms:.1}"))),
            c.compile_ms,
            c.switch_waiting,
            option(c.cache_loaded.map(|bytes| bytes.to_string())),
        );
    }
}

/// Frames [`PerfStats`] keeps by default: ten seconds at 60 fps.
pub const DEFAULT_WINDOW: usize = 600;

/// Frames the overlay's graph shows.
pub const GRAPH_FRAMES: usize = 240;

/// What one frame cost on the CPU, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameSample {
    /// Wall time since the previous frame started: what the player feels (1000 / fps).
    pub frame_ms: f32,
    /// The script phases ([`FrameStats::script_cpu`](crate::FrameStats::script_cpu)).
    pub script_ms: f32,
    /// The simulation steps ([`FrameStats::simulation_cpu`](crate::FrameStats::simulation_cpu)).
    pub physics_ms: f32,
    /// The render hook's own work: scene sync, overlay, recording and submitting the frame. Waiting for a
    /// swapchain image and presenting are not counted (with vsync they are mostly waiting).
    pub render_ms: f32,
    /// The part of `render_ms` spent culling ([`metallic_render::cull`]): the camera's view and the shadow cascades
    /// drawn; 0 when nothing was culled again.
    pub cull_ms: f32,
    /// The part of `render_ms` spent on point and spot lights ([`metallic_render::lights`]): selecting, clustering
    /// and packing them, and culling the casters of the shadow-atlas tiles drawn. [`PerfStats::record`] fills it
    /// from the frame's draws.
    pub lights_ms: f32,
    /// Bit `i` set when shadow cascade `i` was drawn this frame (otherwise its map was kept; see
    /// [`metallic_render::shadow`]). [`PerfStats::record`] fills it from the frame's draws.
    pub shadow_redrawn: u8,
    /// Bit `i` set when cached shadow cascade `i`'s map was built this frame from its static map and its moving
    /// casters. [`PerfStats::record`] fills it from the frame's draws.
    pub shadow_built: u8,
    /// Bit `i` set when that build restored the static map only under its previous moving casters' footprints
    /// rather than copying all of it (#133). [`PerfStats::record`] fills it from the frame's draws.
    pub shadow_restored: u8,
    /// Shadow-atlas tiles drawn this frame ([`metallic_render::lights`]). [`PerfStats::record`] fills it from the
    /// frame's draws.
    pub light_tiles_redrawn: u32,
    /// The render scale the scene was drawn at ([`metallic_render::resolution`]): 1 at the window's resolution.
    pub render_scale: f32,
}

fn ms(d: Duration) -> f32 {
    (d.as_secs_f64() * 1000.0) as f32
}

impl FrameSample {
    /// A sample from durations, with no cull time ([`FrameSample::with_cull`] adds it).
    pub fn new(frame: Duration, script: Duration, physics: Duration, render: Duration) -> Self {
        Self {
            frame_ms: ms(frame),
            script_ms: ms(script),
            physics_ms: ms(physics),
            render_ms: ms(render),
            cull_ms: 0.0,
            lights_ms: 0.0,
            shadow_redrawn: 0,
            shadow_built: 0,
            shadow_restored: 0,
            light_tiles_redrawn: 0,
            render_scale: 1.0,
        }
    }

    /// The sample with `cull` as its cull time.
    pub fn with_cull(self, cull: Duration) -> Self {
        Self {
            cull_ms: ms(cull),
            ..self
        }
    }

    /// The sample drawn at render scale `scale`.
    pub fn with_render_scale(self, scale: f32) -> Self {
        Self {
            render_scale: scale,
            ..self
        }
    }
}

/// The GPU time of one timed frame, per pass, in milliseconds (indexed by [`PassId`] as `usize`; zero for passes
/// that did not run).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuSample {
    /// Each pass's time.
    pub passes_ms: [f32; PassId::COUNT],
    /// Each shadow cascade's time: 0 for a cascade whose map was kept that frame (or that is not in use).
    pub shadow_ms: [f32; MAX_CASCADES],
    /// Each cached shadow cascade's build pass (its static map copied in, its moving casters drawn on top): 0 when
    /// its map was kept that frame (or for a cascade that is not cached).
    pub shadow_build_ms: [f32; MAX_CASCADES],
    /// The local lights' shadow atlas pass: 0 in a frame that kept every tile.
    pub light_shadow_ms: f32,
    /// The sum over the passes, the cascades and the atlas.
    pub total_ms: f32,
}

/// Mean, median, 99th percentile and maximum of one quantity, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Distribution {
    /// The mean.
    pub mean: f32,
    /// The median (50th percentile).
    pub p50: f32,
    /// The 99th percentile.
    pub p99: f32,
    /// The largest value.
    pub max: f32,
}

/// The summary of a [`PerfStats`] window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PerfSummary {
    /// Frames summarised.
    pub frames: usize,
    /// Frame time.
    pub frame_ms: Distribution,
    /// The mean frame time of the slowest 1% of frames (at least one frame).
    pub low_1pct_ms: f32,
    /// Frames over [`PerfSummary::spike_ms`] since the stats were last reset, not counting the first
    /// [`SPIKE_GRACE_MS`] (not only those in the window).
    pub spikes: u32,
    /// What a spike is over: twice the frame target ([`PerfStats::set_frame_target_ms`]).
    pub spike_ms: f32,
    /// The slowest frame since the stats were last reset, not counting the first [`SPIKE_GRACE_MS`].
    pub worst_ms: f32,
    /// Script CPU time.
    pub script_ms: Distribution,
    /// Physics CPU time.
    pub physics_ms: Distribution,
    /// Render CPU time.
    pub render_ms: Distribution,
    /// Cull CPU time (part of the render CPU time).
    pub cull_ms: Distribution,
    /// Local lights' CPU time (part of the render CPU time).
    pub lights_ms: Distribution,
    /// The render scale frames were drawn at (not a time: 1 is the window's resolution).
    pub render_scale: Distribution,
    /// Timed GPU frames summarised.
    pub gpu_frames: usize,
    /// GPU time of the whole frame.
    pub gpu_total_ms: Distribution,
    /// GPU time per pass (indexed by [`PassId`] as `usize`).
    pub gpu_pass_ms: [Distribution; PassId::COUNT],
    /// Which passes ran in any timed frame (bit `PassId as u8`).
    pub gpu_passes: u16,
    /// GPU time per shadow cascade, the frames that kept its map counting 0, so the mean is what it costs per frame
    /// and the median shows whether it is drawn in most frames.
    pub gpu_shadow_ms: [Distribution; MAX_CASCADES],
    /// The most shadow cascades in use in any timed frame: how many of [`PerfSummary::gpu_shadow_ms`] count.
    pub gpu_cascades: usize,
    /// In how many of the summarised frames each shadow cascade was drawn (the others kept their map).
    pub shadow_redraws: [u32; MAX_CASCADES],
    /// GPU time per cached shadow cascade's build pass, the frames that kept its map counting 0.
    pub gpu_shadow_build_ms: [Distribution; MAX_CASCADES],
    /// In how many of the summarised frames each cached shadow cascade's map was built.
    pub shadow_builds: [u32; MAX_CASCADES],
    /// How many of those builds restored footprints rather than copying the whole static map (#133).
    pub shadow_restores: [u32; MAX_CASCADES],
    /// GPU time of the local lights' shadow atlas pass, the frames that kept every tile counting 0.
    pub gpu_light_shadow_ms: Distribution,
    /// In how many of the summarised frames atlas tiles were drawn (the others kept every tile).
    pub light_shadow_frames: u32,
    /// Atlas tiles drawn over the summarised frames.
    pub light_tiles_redrawn: u32,
    /// The last frame's draws, instances and triangles.
    pub draws: DrawStats,
}

impl PerfSummary {
    /// Frames per second at the median frame time.
    pub fn fps_p50(&self) -> f32 {
        fps(self.frame_ms.p50)
    }

    /// Frames per second at the 1% low frame time.
    pub fn low_1pct_fps(&self) -> f32 {
        fps(self.low_1pct_ms)
    }

    /// Whether `pass` ran in any timed frame.
    pub fn gpu_pass_ran(&self, pass: PassId) -> bool {
        self.gpu_passes & (1 << pass as u16) != 0
    }
}

impl PerfSummary {
    /// Writes the summary as the members of a JSON object, without the braces, so a report can put its own
    /// members around them (times in milliseconds): `frames`, `frame_ms`, `fps_p50`, `low_1pct_ms`,
    /// `low_1pct_fps`, `spikes` (`count`, `over_ms`, `worst_ms`), `cpu_ms` (`script`, `physics`, `render`, `cull`, `lights`), `render_scale` (the scale frames
    /// were drawn at), `gpu_ms` (`frames`, `total`, `passes` by name, `shadow` and `shadow_build` per cascade, `light_shadows`), `draws` (with `culled`, and
    /// `levels`: the instances drawn at each level of detail, full detail first), `shadow`
    /// (the last frame's `cascades`, `casters`, `moving` and `restored_texels` per cascade, `draws`, `instances` and
    /// `triangles`, and per cascade the summarised frames its casters were drawn in, `redraws`, its map built in,
    /// `builds`, and built by restoring footprints in, `restores`) and
    /// `lights` (the last frame's light counts, clusters and atlas, and over the
    /// summarised frames `atlas_frames` and `tiles_redrawn`). Each distribution is `{"mean", "p50", "p99", "max"}`.
    pub fn write_json_members(&self, out: &mut String) {
        // Writing to a `String` cannot fail.
        let _ = self.write_members(out);
    }

    fn write_members(&self, out: &mut String) -> std::fmt::Result {
        use std::fmt::Write as _;
        let dist = |d: &Distribution| {
            format!(
                "{{\"mean\":{:.4},\"p50\":{:.4},\"p99\":{:.4},\"max\":{:.4}}}",
                d.mean, d.p50, d.p99, d.max
            )
        };
        write!(out, "\"frames\":{},\"frame_ms\":{},", self.frames, dist(&self.frame_ms))?;
        write!(
            out,
            "\"fps_p50\":{:.1},\"low_1pct_ms\":{:.4},\"low_1pct_fps\":{:.1},",
            self.fps_p50(),
            self.low_1pct_ms,
            self.low_1pct_fps()
        )?;
        write!(
            out,
            "\"spikes\":{{\"count\":{},\"over_ms\":{:.3},\"worst_ms\":{:.4}}},",
            self.spikes, self.spike_ms, self.worst_ms
        )?;
        write!(
            out,
            "\"cpu_ms\":{{\"script\":{},\"physics\":{},\"render\":{},\"cull\":{},\"lights\":{}}},",
            dist(&self.script_ms),
            dist(&self.physics_ms),
            dist(&self.render_ms),
            dist(&self.cull_ms),
            dist(&self.lights_ms)
        )?;
        write!(out, "\"render_scale\":{},", dist(&self.render_scale))?;
        write!(
            out,
            "\"gpu_ms\":{{\"frames\":{},\"total\":{},\"passes\":{{",
            self.gpu_frames,
            dist(&self.gpu_total_ms)
        )?;
        let mut first = true;
        for pass in PassId::ALL {
            if self.gpu_pass_ran(pass) {
                if !first {
                    out.push(',');
                }
                first = false;
                write!(
                    out,
                    "{}:{}",
                    json_string(pass.name()),
                    dist(&self.gpu_pass_ms[pass as usize])
                )?;
            }
        }
        for (name, cascades) in [
            ("},\"shadow\":[", &self.gpu_shadow_ms),
            ("],\"shadow_build\":[", &self.gpu_shadow_build_ms),
        ] {
            out.push_str(name);
            for (i, cascade) in cascades.iter().take(self.gpu_cascades).enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&dist(cascade));
            }
        }
        write!(out, "],\"light_shadows\":{}}},", dist(&self.gpu_light_shadow_ms))?;
        let d = &self.draws;
        write!(
            out,
            "\"draws\":{{\"prepass\":{},\"opaque\":{},\"transparent\":{},\"opaque_instances\":{},\
             \"opaque_triangles\":{},\"transparent_instances\":{},\"transparent_triangles\":{},\"culled\":{},\
             \"levels\":[{},{},{},{}]}},",
            d.prepass_draws,
            d.opaque_draws,
            d.transparent_draws,
            d.opaque_instances,
            d.opaque_triangles,
            d.transparent_instances,
            d.transparent_triangles,
            d.culled_instances,
            d.level_instances[0],
            d.level_instances[1],
            d.level_instances[2],
            d.level_instances[3]
        )?;
        let cascades = d.shadow_cascades as usize;
        let list = |values: &[u32]| values.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        write!(
            out,
            "\"shadow\":{{\"cascades\":{},\"redraws\":[{}],\"builds\":[{}],\"restores\":[{}],\"casters\":[{}],\
             \"moving\":[{}],\"restored_texels\":[{}],\"draws\":{},\"instances\":{},\"triangles\":{}}},",
            cascades,
            list(&self.shadow_redraws[..cascades.min(MAX_CASCADES)]),
            list(&self.shadow_builds[..cascades.min(MAX_CASCADES)]),
            list(&self.shadow_restores[..cascades.min(MAX_CASCADES)]),
            list(&d.shadow_casters[..cascades.min(MAX_CASCADES)]),
            list(&d.shadow_moving[..cascades.min(MAX_CASCADES)]),
            list(&d.shadow_restored_texels[..cascades.min(MAX_CASCADES)]),
            d.shadow_draws,
            d.shadow_instances,
            d.shadow_triangles
        )?;
        let l = &d.lights;
        write!(
            out,
            "\"lights\":{{\"in_scene\":{},\"in_view\":{},\"in_budget\":{},\"drawn\":{},\"fading\":{},\
             \"shadowed\":{},\"shadow_tiles\":{},\"lit_clusters\":{},\"cluster_entries\":{},\"max_per_cluster\":{},\
             \"overflow\":{},\"atlas_draws\":{},\"atlas_instances\":{},\"atlas_triangles\":{},\"atlas_frames\":{},\
             \"tiles_redrawn\":{},\"gpu_bytes\":{},\"directional_shining\":{},\"directional_drawn\":{}}}",
            l.in_scene,
            l.in_view,
            l.in_budget,
            l.drawn,
            l.fading,
            l.shadowed,
            l.shadow_tiles,
            l.lit_clusters,
            l.cluster_entries,
            l.max_per_cluster,
            l.overflow,
            d.light_shadow_draws,
            d.light_shadow_instances,
            d.light_shadow_triangles,
            self.light_shadow_frames,
            self.light_tiles_redrawn,
            l.gpu_bytes,
            l.directional_shining,
            l.directional_drawn
        )
    }
}

/// `s` as a JSON string literal, for reports written by hand (the engine has no JSON dependency).
pub fn json_string(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn fps(ms: f32) -> f32 {
    if ms > 0.0 { 1000.0 / ms } else { 0.0 }
}

/// A fixed-size ring, oldest overwritten first. Never allocates after it is made.
#[derive(Clone, Debug)]
struct Ring<T> {
    items: Vec<T>,
    capacity: usize,
    /// Where the next item goes once the ring is full.
    next: usize,
}

impl<T: Copy> Ring<T> {
    fn new(capacity: usize) -> Self {
        Self {
            items: Vec::with_capacity(capacity),
            capacity,
            next: 0,
        }
    }

    fn push(&mut self, item: T) {
        if self.items.len() < self.capacity {
            self.items.push(item);
        } else if let Some(slot) = self.items.get_mut(self.next) {
            *slot = item;
            self.next = (self.next + 1) % self.capacity;
        }
    }

    fn clear(&mut self) {
        self.items.clear();
        self.next = 0;
    }

    fn len(&self) -> usize {
        self.items.len()
    }

    /// Oldest first.
    fn iter(&self) -> impl Iterator<Item = &T> {
        let (newer, older) = self.items.split_at(self.next.min(self.items.len()));
        older.iter().chain(newer)
    }

    fn last(&self) -> Option<&T> {
        if self.items.len() < self.capacity || self.next == 0 {
            self.items.last()
        } else {
            self.items.get(self.next - 1)
        }
    }
}

/// Collects frame statistics over a window of frames; see the [module documentation](self).
#[derive(Clone, Debug)]
pub struct PerfStats {
    frames: Ring<FrameSample>,
    gpu: Ring<GpuSample>,
    gpu_passes: u16,
    /// The most shadow cascades in use in any timed frame.
    gpu_cascades: usize,
    /// The frame index of the newest GPU timings recorded, so each timed frame counts once.
    last_gpu_frame: Option<u64>,
    draws: DrawStats,
    scratch: Vec<f32>,
    recorded: u64,
    /// The frame time a frame should take; a frame over twice it is a spike.
    target_ms: f32,
    /// Wall time recorded since the stats were made (a reset keeps it): spikes count after [`SPIKE_GRACE_MS`].
    elapsed_ms: f64,
    /// Spikes since the last reset, after the grace.
    spikes: u32,
    /// The slowest frame since the last reset, after the grace.
    worst_ms: f32,
}

/// How long after start frames are not counted as spikes: loading, the first uploads, the pipelines compiled in the
/// background (ADR 0008 §9: no frame over twice the target after the first 2 s).
pub const SPIKE_GRACE_MS: f64 = 2000.0;

/// The frame target spikes are measured against unless the app sets one: a 60 fps frame.
pub const DEFAULT_FRAME_TARGET_MS: f32 = 1000.0 / 60.0;

impl Default for PerfStats {
    fn default() -> Self {
        Self::new(DEFAULT_WINDOW)
    }
}

impl PerfStats {
    /// Statistics over the last `window` frames (at least one).
    pub fn new(window: usize) -> Self {
        let window = window.max(1);
        Self {
            frames: Ring::new(window),
            gpu: Ring::new(window),
            gpu_passes: 0,
            gpu_cascades: 0,
            last_gpu_frame: None,
            draws: DrawStats::default(),
            scratch: Vec::with_capacity(window),
            recorded: 0,
            target_ms: DEFAULT_FRAME_TARGET_MS,
            elapsed_ms: 0.0,
            spikes: 0,
            worst_ms: 0.0,
        }
    }

    /// Sets the frame time frames should take (a frame cap's, or the display's under vsync; a 60 fps frame by
    /// default): a frame over twice it, after the first [`SPIKE_GRACE_MS`], is a spike.
    pub fn set_frame_target_ms(&mut self, ms: f32) {
        if ms.is_finite() && ms > 0.0 {
            self.target_ms = ms;
        }
    }

    /// The frame time frames should take.
    pub fn frame_target_ms(&self) -> f32 {
        self.target_ms
    }

    /// Frames over twice the target since the last reset, not counting the first [`SPIKE_GRACE_MS`] after the stats
    /// were made.
    pub fn spikes(&self) -> u32 {
        self.spikes
    }

    /// The slowest frame since the last reset, not counting the first [`SPIKE_GRACE_MS`]; 0 before any.
    pub fn worst_frame_ms(&self) -> f32 {
        self.worst_ms
    }

    /// Frames the window holds at most.
    pub fn window(&self) -> usize {
        self.frames.capacity
    }

    /// Records a frame: its CPU sample, the newest GPU timings the renderer has (recorded only if they are for a
    /// frame not seen yet) and what it drew.
    pub fn record(&mut self, sample: FrameSample, gpu: Option<&FrameTimings>, draws: DrawStats) {
        self.frames.push(FrameSample {
            shadow_redrawn: draws.shadow_redrawn,
            shadow_built: draws.shadow_built,
            shadow_restored: draws.shadow_restored,
            light_tiles_redrawn: draws.lights.tiles_redrawn,
            lights_ms: ms(draws.lights.total_cpu_time()),
            ..sample
        });
        self.draws = draws;
        self.recorded += 1;
        if self.elapsed_ms >= SPIKE_GRACE_MS {
            self.worst_ms = self.worst_ms.max(sample.frame_ms);
            self.spikes += u32::from(sample.frame_ms > 2.0 * self.target_ms);
        }
        self.elapsed_ms += f64::from(sample.frame_ms);
        let Some(timings) = gpu else {
            return;
        };
        if self.last_gpu_frame.is_some_and(|frame| frame >= timings.frame()) {
            return;
        }
        self.last_gpu_frame = Some(timings.frame());
        let mut gpu = GpuSample::default();
        for pass in PassId::ALL {
            if let Some(ms) = timings.pass(pass) {
                gpu.passes_ms[pass as usize] = ms;
                gpu.total_ms += ms;
                self.gpu_passes |= 1 << pass as u16;
            }
        }
        for cascade in 0..timings.cascade_count().min(MAX_CASCADES) {
            let ms = timings.cascade(cascade).unwrap_or(0.0);
            gpu.shadow_ms[cascade] = ms;
            let build = timings.cascade_build(cascade).unwrap_or(0.0);
            gpu.shadow_build_ms[cascade] = build;
            gpu.total_ms += ms + build;
        }
        gpu.light_shadow_ms = timings.light_shadow_ms();
        gpu.total_ms += gpu.light_shadow_ms;
        self.gpu_cascades = self.gpu_cascades.max(timings.cascade_count().min(MAX_CASCADES));
        self.gpu.push(gpu);
    }

    /// Forgets every frame (after a warm-up, or when the scene changes), and the spikes. GPU timings arrive late, so
    /// the first one or two recorded after a reset can be of frames rendered just before it.
    pub fn reset(&mut self) {
        self.frames.clear();
        self.gpu.clear();
        self.gpu_passes = 0;
        self.gpu_cascades = 0;
        self.draws = DrawStats::default();
        self.recorded = 0;
        self.spikes = 0;
        self.worst_ms = 0.0;
    }

    /// Frames recorded since the last reset (more than the window holds, once it wraps).
    pub fn recorded(&self) -> u64 {
        self.recorded
    }

    /// Frames in the window, oldest first.
    pub fn frames(&self) -> impl Iterator<Item = &FrameSample> {
        self.frames.iter()
    }

    /// Timed GPU frames in the window, oldest first.
    pub fn gpu_frames(&self) -> impl Iterator<Item = &GpuSample> {
        self.gpu.iter()
    }

    /// The newest frame.
    pub fn last(&self) -> Option<&FrameSample> {
        self.frames.last()
    }

    /// The newest GPU timings.
    pub fn last_gpu(&self) -> Option<&GpuSample> {
        self.gpu.last()
    }

    /// The last frame's draws.
    pub fn draws(&self) -> DrawStats {
        self.draws
    }

    /// Summarises the window. Sorts a copy of each quantity in a buffer kept for it, so it allocates nothing, but
    /// it is not free (a few sorts of `window` numbers): an overlay refreshes it a few times a second.
    pub fn summary(&mut self) -> PerfSummary {
        let mut summary = PerfSummary {
            frames: self.frames.len(),
            spikes: self.spikes,
            spike_ms: 2.0 * self.target_ms,
            worst_ms: self.worst_ms,
            gpu_frames: self.gpu.len(),
            gpu_passes: self.gpu_passes,
            gpu_cascades: self.gpu_cascades,
            draws: self.draws,
            ..PerfSummary::default()
        };
        for frame in self.frames.iter() {
            for cascade in 0..MAX_CASCADES {
                summary.shadow_redraws[cascade] += u32::from(frame.shadow_redrawn >> cascade & 1);
                summary.shadow_builds[cascade] += u32::from(frame.shadow_built >> cascade & 1);
                summary.shadow_restores[cascade] += u32::from(frame.shadow_restored >> cascade & 1);
            }
            summary.light_shadow_frames += u32::from(frame.light_tiles_redrawn > 0);
            summary.light_tiles_redrawn += frame.light_tiles_redrawn;
        }
        let (frames, gpu, scratch) = (&self.frames, &self.gpu, &mut self.scratch);
        summary.frame_ms = distribution(scratch, frames.iter().map(|f| f.frame_ms));
        // The slowest 1% are the end of the sorted frame times left in the scratch buffer.
        let slowest = scratch.len().div_ceil(100);
        if slowest > 0 {
            let tail = &scratch[scratch.len() - slowest..];
            summary.low_1pct_ms = tail.iter().sum::<f32>() / slowest as f32;
        }
        summary.script_ms = distribution(scratch, frames.iter().map(|f| f.script_ms));
        summary.physics_ms = distribution(scratch, frames.iter().map(|f| f.physics_ms));
        summary.render_ms = distribution(scratch, frames.iter().map(|f| f.render_ms));
        summary.cull_ms = distribution(scratch, frames.iter().map(|f| f.cull_ms));
        summary.lights_ms = distribution(scratch, frames.iter().map(|f| f.lights_ms));
        summary.render_scale = distribution(scratch, frames.iter().map(|f| f.render_scale));
        summary.gpu_total_ms = distribution(scratch, gpu.iter().map(|g| g.total_ms));
        summary.gpu_light_shadow_ms = distribution(scratch, gpu.iter().map(|g| g.light_shadow_ms));
        for pass in PassId::ALL {
            if summary.gpu_pass_ran(pass) {
                let i = pass as usize;
                summary.gpu_pass_ms[i] = distribution(scratch, gpu.iter().map(|g| g.passes_ms[i]));
            }
        }
        for cascade in 0..summary.gpu_cascades {
            summary.gpu_shadow_ms[cascade] = distribution(scratch, gpu.iter().map(|g| g.shadow_ms[cascade]));
            summary.gpu_shadow_build_ms[cascade] =
                distribution(scratch, gpu.iter().map(|g| g.shadow_build_ms[cascade]));
        }
        summary
    }
}

/// Fills `scratch` with `values`, sorts it and reads the distribution. `scratch` keeps the sorted values.
fn distribution(scratch: &mut Vec<f32>, values: impl Iterator<Item = f32>) -> Distribution {
    scratch.clear();
    // The scratch buffer has the window's capacity and no ring holds more, so this never allocates.
    scratch.extend(values);
    if scratch.is_empty() {
        return Distribution::default();
    }
    scratch.sort_unstable_by(f32::total_cmp);
    let at = |p: f64| scratch[((scratch.len() - 1) as f64 * p).round() as usize];
    Distribution {
        mean: scratch.iter().sum::<f32>() / scratch.len() as f32,
        p50: at(0.5),
        p99: at(0.99),
        max: scratch[scratch.len() - 1],
    }
}

/// Overlay colours (sRGB, straight alpha).
const PANEL: [u8; 4] = [8, 10, 14, 200];
const TEXT: [u8; 4] = [235, 238, 242, 255];
const DIM: [u8; 4] = [150, 160, 172, 255];
const GOOD: [u8; 4] = [96, 214, 120, 255];
const SLOW: [u8; 4] = [240, 196, 70, 255];
const BAD: [u8; 4] = [240, 90, 80, 255];
const GUIDE: [u8; 4] = [255, 255, 255, 70];

/// Text scale of the overlay (font pixels to screen pixels).
const SCALE: f32 = 2.0;
/// The graph's height in pixels; its top is 33.3 ms (30 fps).
const GRAPH_HEIGHT: f32 = 100.0;
const GRAPH_TOP_MS: f32 = 1000.0 / 30.0;
/// Width of one frame's bar.
const BAR: f32 = 2.0;

fn bar_color(ms: f32) -> [u8; 4] {
    if ms <= 1000.0 / 60.0 {
        GOOD
    } else if ms <= 1000.0 / 30.0 {
        SLOW
    } else {
        BAD
    }
}

/// A preset's or tier's name in the overlay's capitals.
fn upper(name: &str) -> &'static str {
    match name {
        "low" => "LOW",
        "medium" => "MEDIUM",
        "high" => "HIGH",
        "ultra" => "ULTRA",
        "auto" => "AUTO",
        _ => "?",
    }
}

/// The overlay's graphics line: the preset and tier (or what calibration measures), the scale and size drawn, and
/// dynamic resolution's range and target with the median scale of the summarised frames.
fn draw_graphics_line(overlay: &mut Overlay, x0: f32, y: f32, graphics: &GraphicsStatus, summary: &PerfSummary) {
    let r = &graphics.resolution;
    let preset = graphics.preset.map_or("DEFAULTS", |p| upper(p.name()));
    let (mut x, _) = match (graphics.calibrating, graphics.tier) {
        (Some(probe), _) => overlay.text_fmt(
            x0,
            y,
            SCALE,
            SLOW,
            format_args!("GRAPHICS {preset}  CALIBRATING {}", upper(probe.name())),
        ),
        (None, Some(tier)) if graphics.preset.and_then(Preset::tier) != Some(tier) => overlay.text_fmt(
            x0,
            y,
            SCALE,
            TEXT,
            format_args!("GRAPHICS {preset} ({})", upper(tier.name())),
        ),
        _ => overlay.text_fmt(x0, y, SCALE, TEXT, format_args!("GRAPHICS {preset}")),
    };
    (x, _) = overlay.text_fmt(
        x,
        y,
        SCALE,
        TEXT,
        format_args!("  SCALE {:.2} {}X{}", r.scale, r.width, r.height),
    );
    if r.dynamic {
        overlay.text_fmt(
            x,
            y,
            SCALE,
            DIM,
            format_args!(
                "  DYNAMIC {:.2}-{:.2} TARGET {:.2} MS P50 {:.2}",
                r.min_scale, r.max_scale, r.target_ms, summary.render_scale.p50
            ),
        );
    }
}

/// Clears `overlay` and draws the performance overlay at its top-left corner: frame time with p50, p99 and the
/// 1% low; script, physics, render and cull CPU time; GPU time per pass; the shadow cascades; the point and spot
/// lights (in the scene, in view, drawn, fading, shadowed) and their CPU and atlas cost; draws, instances and
/// triangles per pass and the instances culled; the graphics settings in effect (`graphics`: the preset and its
/// tier, or Auto's calibration while it runs, the render scale and size drawn, and dynamic resolution's range and
/// target); the pipelines compiled and still compiling, and the spike counter (frames over twice the frame target
/// since the first 2 s, and the slowest); and a graph of the last [`GRAPH_FRAMES`] frame times (green within 60 fps,
/// yellow within 30, red beyond; guides at 16.7 and 33.3 ms). `summary` is usually refreshed a few times a second;
/// the graph, the spikes and the "now" numbers come from `stats` every frame. Allocates nothing.
pub fn draw_overlay(overlay: &mut Overlay, stats: &PerfStats, summary: &PerfSummary, graphics: &GraphicsStatus) {
    overlay.clear();
    let (x0, y0) = (12.0, 12.0);
    let line = metallic_render::overlay::LINE_HEIGHT * SCALE;
    // Wide enough for the graph and for the longest line of text (about 74 characters).
    let width = (GRAPH_FRAMES as f32 * BAR).max(76.0 * metallic_render::overlay::GLYPH_ADVANCE * SCALE) + 16.0;
    let lines = 15.0;
    overlay.rect(x0 - 8.0, y0 - 8.0, width, lines * line + GRAPH_HEIGHT + 24.0, PANEL);

    let now = stats.last().copied().unwrap_or_default();
    let mut y = y0;
    overlay.text_fmt(
        x0,
        y,
        SCALE,
        bar_color(summary.frame_ms.p50),
        format_args!(
            "FRAME {:.2} MS ({:.0} FPS)  P50 {:.2}  P99 {:.2}  1% LOW {:.2}",
            now.frame_ms,
            fps(now.frame_ms),
            summary.frame_ms.p50,
            summary.frame_ms.p99,
            summary.low_1pct_ms,
        ),
    );
    y += line;
    draw_graphics_line(overlay, x0, y, graphics, summary);
    y += line;
    let post = &graphics.post;
    overlay.text_fmt(
        x0,
        y,
        SCALE,
        TEXT,
        format_args!(
            "POST  {}  AA {}  BLOOM {}  PASSES {} ({} FULL-SCREEN)",
            post.format_name(),
            post.anti_aliasing().name(),
            post.bloom_levels,
            post.render_passes,
            post.full_screen_passes
        ),
    );
    y += line;
    // Pipelines compiled and still compiling (background compiles, ADR 0008 §9), then the frames over twice the
    // frame target since the first 2 s, and the slowest of them.
    let pipelines = &graphics.pipelines;
    let (x, _) = overlay.text_fmt(
        x0,
        y,
        SCALE,
        if pipelines.pending > 0 { SLOW } else { TEXT },
        format_args!(
            "PIPELINES {} ({} PENDING) FIRST {:.0} MS",
            pipelines.compiled, pipelines.pending, pipelines.first_frame_ms
        ),
    );
    overlay.text_fmt(
        x,
        y,
        SCALE,
        if stats.spikes() > 0 { BAD } else { TEXT },
        format_args!(
            "  SPIKES {} OVER {:.1} MS  WORST {:.2}",
            stats.spikes(),
            2.0 * stats.frame_target_ms(),
            stats.worst_frame_ms()
        ),
    );
    y += line;
    overlay.text_fmt(
        x0,
        y,
        SCALE,
        TEXT,
        format_args!(
            "CPU   SCRIPT {:.2}  PHYSICS {:.2}  RENDER {:.2} (CULL {:.2}) MS (P50)",
            summary.script_ms.p50, summary.physics_ms.p50, summary.render_ms.p50, summary.cull_ms.p50
        ),
    );
    y += line;
    let (mut x, _) = overlay.text_fmt(
        x0,
        y,
        SCALE,
        TEXT,
        format_args!("GPU   {:.2} MS", summary.gpu_total_ms.p50),
    );
    for pass in PassId::ALL {
        if summary.gpu_pass_ran(pass) {
            (x, _) = overlay.text_fmt(
                x,
                y,
                SCALE,
                DIM,
                format_args!("  {} {:.2}", pass.name(), summary.gpu_pass_ms[pass as usize].p50),
            );
        }
    }
    y += line;
    // Shadow cascades: GPU time per cascade (the mean, which counts the frames that kept a map as 0, with a cached
    // cascade's build passes, and the share of frames that drew its casters), then what the last frame drew and each
    // map's casters.
    let (mut x, _) = overlay.text_fmt(x0, y, SCALE, TEXT, format_args!("SHADOW"));
    for cascade in 0..summary.gpu_cascades.min(MAX_CASCADES) {
        let drawn = summary.shadow_redraws[cascade] as f32 / summary.frames.max(1) as f32;
        (x, _) = overlay.text_fmt(
            x,
            y,
            SCALE,
            DIM,
            format_args!(
                "  C{cascade} {:.2} MS {:3.0}%",
                summary.gpu_shadow_ms[cascade].mean + summary.gpu_shadow_build_ms[cascade].mean,
                drawn * 100.0
            ),
        );
    }
    y += line;
    let draws = summary.draws;
    let casters = &draws.shadow_casters[..(draws.shadow_cascades as usize).min(MAX_CASCADES)];
    let (mut x, _) = overlay.text_fmt(
        x0,
        y,
        SCALE,
        TEXT,
        format_args!(
            "SHADOW {:6} DRAWS {:7} INST  CASTERS",
            draws.shadow_draws, draws.shadow_instances
        ),
    );
    for &n in casters {
        (x, _) = overlay.text_fmt(x, y, SCALE, DIM, format_args!(" {n}"));
    }
    y += line;
    // Point and spot lights: how many there are, are drawn and shadowed, then what they cost.
    let lights = &draws.lights;
    overlay.text_fmt(
        x0,
        y,
        SCALE,
        TEXT,
        format_args!(
            "LIGHTS {:5} IN SCENE {:4} IN VIEW {:4} DRAWN ({} FADING) {:3} SHADOWED",
            lights.in_scene, lights.in_view, lights.drawn, lights.fading, lights.shadowed
        ),
    );
    y += line;
    let atlas_share = summary.light_shadow_frames as f32 / summary.frames.max(1) as f32;
    overlay.text_fmt(
        x0,
        y,
        SCALE,
        DIM,
        format_args!(
            "LIGHTS CPU {:.2} MS  ATLAS {:.2} MS {:3.0}%  {} TILES  {} PER CLUSTER MAX",
            summary.lights_ms.p50,
            summary.gpu_light_shadow_ms.mean,
            atlas_share * 100.0,
            lights.shadow_tiles,
            lights.max_per_cluster
        ),
    );
    y += line;
    overlay.text_fmt(
        x0,
        y,
        SCALE,
        TEXT,
        format_args!(
            "OPAQUE      {:4} DRAWS {:7} INST {:8.3} M TRI",
            draws.opaque_draws,
            draws.opaque_instances,
            draws.opaque_triangles as f64 / 1e6
        ),
    );
    y += line;
    overlay.text_fmt(
        x0,
        y,
        SCALE,
        TEXT,
        format_args!(
            "TRANSPARENT {:4} DRAWS {:7} INST {:8.3} M TRI",
            draws.transparent_draws,
            draws.transparent_instances,
            draws.transparent_triangles as f64 / 1e6
        ),
    );
    y += line;
    overlay.text_fmt(
        x0,
        y,
        SCALE,
        DIM,
        format_args!("CULLED            {:7} INST", draws.culled_instances),
    );
    y += line;
    if draws.prepass_draws > 0 {
        overlay.text_fmt(
            x0,
            y,
            SCALE,
            TEXT,
            format_args!(
                "PREPASS     {:4} DRAWS {:7} INST {:8.3} M TRI",
                draws.prepass_draws,
                draws.opaque_instances,
                draws.opaque_triangles as f64 / 1e6
            ),
        );
    }
    y += line;
    overlay.text_fmt(
        x0,
        y,
        SCALE,
        DIM,
        format_args!("{} FRAMES (P50/P99 OVER THE LAST {})", stats.recorded(), summary.frames),
    );
    y += line + 4.0;

    // The graph: one bar per frame, newest on the right.
    let bottom = y + GRAPH_HEIGHT;
    let skip = stats.frames.len().saturating_sub(GRAPH_FRAMES);
    let first = GRAPH_FRAMES - (stats.frames.len() - skip);
    for (i, frame) in stats.frames().skip(skip).enumerate() {
        let h = (frame.frame_ms / GRAPH_TOP_MS).min(1.0) * GRAPH_HEIGHT;
        let x = x0 + (first + i) as f32 * BAR;
        overlay.rect(x, bottom - h.max(1.0), BAR, h.max(1.0), bar_color(frame.frame_ms));
    }
    for guide_ms in [1000.0 / 60.0, 1000.0 / 30.0] {
        let gy = bottom - (guide_ms / GRAPH_TOP_MS).min(1.0) * GRAPH_HEIGHT;
        overlay.rect(x0, gy, GRAPH_FRAMES as f32 * BAR, 1.0, GUIDE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(frame_ms: f32) -> FrameSample {
        FrameSample {
            frame_ms,
            script_ms: frame_ms / 10.0,
            physics_ms: 0.0,
            render_ms: frame_ms / 2.0,
            cull_ms: frame_ms / 20.0,
            shadow_redrawn: 0,
            ..FrameSample::default()
        }
    }

    #[test]
    fn percentiles_and_the_one_percent_low() {
        let mut stats = PerfStats::new(1000);
        // 1..=1000 ms: p50 is the 500th or 501st value, p99 the 990th, and the 1% low the mean of the ten slowest.
        for ms in 1..=1000 {
            stats.record(sample(ms as f32), None, DrawStats::default());
        }
        let summary = stats.summary();
        assert_eq!(summary.frames, 1000);
        assert_eq!(summary.frame_ms.p50, 501.0);
        assert_eq!(summary.frame_ms.p99, 990.0);
        assert_eq!(summary.frame_ms.max, 1000.0);
        assert!((summary.frame_ms.mean - 500.5).abs() < 1e-3);
        assert!((summary.low_1pct_ms - 995.5).abs() < 1e-3);
        assert!((summary.low_1pct_fps() - 1000.0 / 995.5).abs() < 1e-6);
        assert_eq!(summary.render_ms.p50, 250.5);
        assert_eq!(summary.gpu_frames, 0);
    }

    #[test]
    fn spikes_are_frames_over_twice_the_target_after_the_first_two_seconds() {
        let mut stats = PerfStats::new(16);
        stats.set_frame_target_ms(10.0);
        // The first 2 s: a 50 ms frame here is loading, not a spike.
        for _ in 0..19 {
            stats.record(sample(100.0), None, DrawStats::default());
        }
        stats.record(sample(50.0), None, DrawStats::default());
        assert_eq!((stats.spikes(), stats.worst_frame_ms()), (0, 0.0));
        // 1,950 ms so far; this frame starts before 2 s too.
        stats.record(sample(60.0), None, DrawStats::default());
        assert_eq!(stats.spikes(), 0);
        stats.record(sample(20.0), None, DrawStats::default());
        stats.record(sample(20.5), None, DrawStats::default());
        stats.record(sample(9.0), None, DrawStats::default());
        assert_eq!((stats.spikes(), stats.worst_frame_ms()), (1, 20.5));
        let summary = stats.summary();
        assert_eq!((summary.spikes, summary.spike_ms, summary.worst_ms), (1, 20.0, 20.5));
        // A reset (the end of a warm-up) clears them; the grace is not given again.
        stats.reset();
        assert_eq!((stats.spikes(), stats.worst_frame_ms()), (0, 0.0));
        stats.record(sample(30.0), None, DrawStats::default());
        assert_eq!(stats.spikes(), 1);
        // Nonsense targets are ignored.
        stats.set_frame_target_ms(0.0);
        stats.set_frame_target_ms(f32::NAN);
        assert_eq!(stats.frame_target_ms(), 10.0);
    }

    #[test]
    fn the_window_keeps_the_newest_frames() {
        let mut stats = PerfStats::new(4);
        for ms in 1..=6 {
            stats.record(sample(ms as f32), None, DrawStats::default());
        }
        let kept: Vec<f32> = stats.frames().map(|f| f.frame_ms).collect();
        assert_eq!(kept, [3.0, 4.0, 5.0, 6.0]);
        assert_eq!(stats.last().map(|f| f.frame_ms), Some(6.0));
        assert_eq!(stats.recorded(), 6);
        assert_eq!(stats.summary().frame_ms.max, 6.0);
        // One frame: the 1% low is that frame.
        stats.reset();
        stats.record(sample(7.0), None, DrawStats::default());
        assert_eq!(stats.summary().low_1pct_ms, 7.0);
        assert_eq!(stats.recorded(), 1);
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_string("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
    }

    #[test]
    fn an_empty_window_summarises_to_zero() {
        let mut stats = PerfStats::new(8);
        assert_eq!(
            stats.summary(),
            PerfSummary {
                spike_ms: 2.0 * DEFAULT_FRAME_TARGET_MS,
                ..PerfSummary::default()
            }
        );
        assert_eq!(stats.summary().fps_p50(), 0.0);
        let mut overlay = Overlay::new();
        draw_overlay(
            &mut overlay,
            &stats,
            &stats.clone().summary(),
            &GraphicsStatus::default(),
        );
        assert!(!overlay.is_empty());
        assert_eq!(overlay.dropped(), 0);
    }

    #[test]
    fn the_overlay_fits_its_storage() {
        let mut stats = PerfStats::default();
        for i in 0..DEFAULT_WINDOW {
            stats.record(sample((i % 50) as f32), None, DrawStats::default());
        }
        let summary = stats.summary();
        let mut overlay = Overlay::new();
        draw_overlay(&mut overlay, &stats, &summary, &GraphicsStatus::default());
        assert_eq!(overlay.dropped(), 0);
        assert!(overlay.len() > GRAPH_FRAMES, "a bar per frame plus the text");
    }
}
