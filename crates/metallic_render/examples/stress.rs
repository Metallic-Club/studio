//! Stress test: 5,000 random Parts on a baseplate in a 1920x1080 window.
//!
//! Prints average and minimum fps every 5 seconds.
//!
//! Options: `--seconds N` exit after N seconds and print the overall average,
//! `--vsync` present with vsync (default is uncapped so the number means throughput),
//! `--no-prewarm` skip the pre-warm frame (to measure first-frame cost),
//! `--low` use the Low quality preset,
//! `--screenshot PATH` render one frame offscreen (no window) to a PNG and exit.
use metallic_core::{Color3, InstanceId, Value, Vec3 as V3, World};
use metallic_render::glam::Vec3;
use metallic_render::{Camera, Quality, RenderContext, RenderSettings, Renderer};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

const PART_COUNT: usize = 5000;
/// Parts moved through `World::set` every frame, so the fps includes the sync cost.
const MOVING_PARTS: usize = 500;

/// Tiny deterministic random generator so every run draws the same scene.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() * items.len() as f32) as usize % items.len()]
    }
}

fn build_world() -> World {
    const MATERIALS: [&str; 10] = [
        "Plastic",
        "SmoothPlastic",
        "Metal",
        "Wood",
        "Concrete",
        "Glass",
        "Grass",
        "Brick",
        "Ice",
        "Sand",
    ];
    const SHAPES: [&str; 4] = ["Block", "Ball", "Cylinder", "Wedge"];
    // Graphite, silver and violet with a few cool and warm accents.
    const PALETTE: [(f64, f64, f64); 10] = [
        (196.0, 200.0, 210.0),
        (150.0, 154.0, 166.0),
        (88.0, 90.0, 102.0),
        (52.0, 54.0, 64.0),
        (139.0, 108.0, 214.0),
        (110.0, 84.0, 180.0),
        (184.0, 164.0, 232.0),
        (92.0, 140.0, 178.0),
        (214.0, 152.0, 96.0),
        (230.0, 232.0, 238.0),
    ];
    let mut w = World::new_with_services();
    let Some(ws) = w.root("Workspace") else {
        return w;
    };
    let part =
        |w: &mut World, pos: V3, size: V3, color: (f64, f64, f64), mat: &str, shape: &str| {
            let Ok(p) = w.create("Part", Some(ws)) else {
                return;
            };
            let _ = w.set(p, "Position", Value::Vector3(pos));
            let _ = w.set(p, "Size", Value::Vector3(size));
            let _ = w.set(
                p,
                "Color",
                Value::Color3(Color3::new(color.0, color.1, color.2)),
            );
            let _ = w.set(p, "Material", Value::String(mat.into()));
            let _ = w.set(p, "Shape", Value::String(shape.into()));
            let _ = w.set(p, "Anchored", Value::Bool(true));
        };
    part(
        &mut w,
        V3::new(0.0, -0.5, 0.0),
        V3::new(2048.0, 1.0, 2048.0),
        (84.0, 86.0, 98.0),
        "Concrete",
        "Block",
    );
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    for _ in 0..PART_COUNT {
        let size = V3::new(
            f64::from(rng.range(2.0, 10.0)),
            f64::from(rng.range(1.0, 12.0)),
            f64::from(rng.range(2.0, 10.0)),
        );
        let pos = V3::new(
            f64::from(rng.range(-230.0, 230.0)),
            size.y / 2.0 + f64::from(rng.range(0.0, 3.0)),
            f64::from(rng.range(-230.0, 230.0)),
        );
        let color = *rng.pick(&PALETTE);
        let mat = *rng.pick(&MATERIALS);
        let shape = *rng.pick(&SHAPES);
        part(&mut w, pos, size, color, mat, shape);
        // Rotate and make about one part in twelve see-through. The new part is the last child.
        if let Some(&last) = w.children(ws).last() {
            let _ = w.set(
                last,
                "Orientation",
                Value::Vector3(V3::new(0.0, f64::from(rng.range(0.0, 360.0)), 0.0)),
            );
            if rng.next() < 0.085 {
                let _ = w.set(
                    last,
                    "Transparency",
                    Value::Number(f64::from(rng.range(0.3, 0.7))),
                );
            }
        }
    }
    w
}

/// The first `MOVING_PARTS` parts after the baseplate, with their start positions.
fn collect_movers(world: &World) -> Vec<(InstanceId, V3)> {
    let Some(ws) = world.root("Workspace") else {
        return Vec::new();
    };
    world
        .children(ws)
        .iter()
        .skip(1)
        .take(MOVING_PARTS)
        .filter_map(|&id| match world.get(id, "Position") {
            Ok(Value::Vector3(p)) => Some((id, p)),
            _ => None,
        })
        .collect()
}

fn orbit_camera(t: f32) -> Camera {
    let a = t * 0.15;
    Camera {
        position: Vec3::new(130.0 * a.cos(), 55.0, 130.0 * a.sin()),
        look_at: Vec3::new(0.0, 4.0, 0.0),
        fov_deg: 70.0,
    }
}

/// Frame times kept for the percentiles (about a minute at 160 fps), so a long run uses
/// bounded memory.
const RECENT_FRAMES: usize = 10_000;
/// How many of the slowest frames the summary names.
const WORST_FRAMES: usize = 5;

/// Running frame-time statistics: totals over the whole run plus a bounded ring of recent
/// frame times for the percentiles.
struct FrameTimes {
    /// The last `RECENT_FRAMES` frame times in milliseconds, oldest first.
    recent: VecDeque<f32>,
    count: u64,
    over_33ms: u64,
    /// The slowest frames of the run as (frame index, ms), slowest first.
    worst: Vec<(u64, f32)>,
}

impl FrameTimes {
    fn new() -> FrameTimes {
        FrameTimes {
            recent: VecDeque::with_capacity(RECENT_FRAMES),
            count: 0,
            over_33ms: 0,
            worst: Vec::with_capacity(WORST_FRAMES + 1),
        }
    }

    fn push(&mut self, ms: f32) {
        if self.recent.len() == RECENT_FRAMES {
            self.recent.pop_front();
        }
        self.recent.push_back(ms);
        if ms > 33.0 {
            self.over_33ms += 1;
        }
        let at = self.worst.partition_point(|&(_, w)| w >= ms);
        if at < WORST_FRAMES {
            self.worst.insert(at, (self.count, ms));
            self.worst.truncate(WORST_FRAMES);
        }
        self.count += 1;
    }

    /// The `q` quantile (0..=1) of the recent frame times.
    fn recent_quantile(&self, q: f32) -> f32 {
        let mut sorted: Vec<f32> = self.recent.iter().copied().collect();
        sorted.sort_by(f32::total_cmp);
        sorted
            .get(((sorted.len() as f32 - 1.0) * q) as usize)
            .copied()
            .unwrap_or(0.0)
    }
}

struct Stats {
    window_start: Instant,
    frames: u32,
    worst: Duration,
    last_frame: Instant,
    total_start: Instant,
    total_frames: u64,
    sync_total: Duration,
    sync_window: Duration,
    times: FrameTimes,
}

impl Stats {
    fn new() -> Stats {
        let now = Instant::now();
        Stats {
            window_start: now,
            frames: 0,
            worst: Duration::ZERO,
            last_frame: now,
            total_start: now,
            total_frames: 0,
            sync_total: Duration::ZERO,
            sync_window: Duration::ZERO,
            times: FrameTimes::new(),
        }
    }

    fn add_sync(&mut self, d: Duration) {
        self.sync_total += d;
        self.sync_window += d;
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = now - self.last_frame;
        self.worst = self.worst.max(dt);
        self.times.push(dt.as_secs_f32() * 1000.0);
        self.last_frame = now;
        self.frames += 1;
        self.total_frames += 1;
        let elapsed = now - self.window_start;
        if elapsed >= Duration::from_secs(5) {
            println!(
                "fps avg {:.1}  min {:.1}  sync {:.3} ms/frame  ({} frames in {:.1}s)",
                f64::from(self.frames) / elapsed.as_secs_f64(),
                1.0 / self.worst.as_secs_f64(),
                self.sync_window.as_secs_f64() * 1000.0 / f64::from(self.frames),
                self.frames,
                elapsed.as_secs_f64()
            );
            self.window_start = now;
            self.frames = 0;
            self.worst = Duration::ZERO;
            self.sync_window = Duration::ZERO;
        }
    }

    fn overall(&self) -> f64 {
        self.total_frames as f64 / self.total_start.elapsed().as_secs_f64()
    }

    fn summary(&self) {
        let t = &self.times;
        let slow: Vec<String> = t
            .worst
            .iter()
            .map(|(i, ms)| format!("#{i}={ms:.1}ms"))
            .collect();
        println!(
            "overall fps avg {:.1} over {} frames; frame ms (last {}) p50 {:.2} p99 {:.2} p99.9 {:.2}; max {:.2}; >33 ms: {} frames; worst: {}; sync avg {:.3} ms/frame",
            self.overall(),
            t.count,
            t.recent.len(),
            t.recent_quantile(0.5),
            t.recent_quantile(0.99),
            t.recent_quantile(0.999),
            t.worst.first().map_or(0.0, |w| w.1),
            t.over_33ms,
            slow.join(" "),
            self.sync_total.as_secs_f64() * 1000.0 / t.count.max(1) as f64,
        );
    }
}

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    ctx: RenderContext,
    renderer: Renderer,
    minimised: bool,
}

struct App {
    gfx: Option<Gfx>,
    world: World,
    movers: Vec<(InstanceId, V3)>,
    no_prewarm: bool,
    settings: RenderSettings,
    vsync: bool,
    seconds: Option<f32>,
    stats: Stats,
    start: Instant,
}

impl App {
    fn render(&mut self, event_loop: &ActiveEventLoop) {
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        if gfx.minimised || gfx.config.width == 0 || gfx.config.height == 0 {
            return;
        }
        let t = self.start.elapsed().as_secs_f32();
        if let Some(limit) = self.seconds
            && t >= limit
        {
            self.stats.summary();
            event_loop.exit();
            return;
        }
        // Move 500 parts through the world and sync, like a running game would.
        let sync_start = Instant::now();
        for (i, (id, base)) in self.movers.iter().enumerate() {
            let phase = t * 1.5 + i as f32 * 0.37;
            let pos = V3::new(
                base.x + f64::from(phase.sin()) * 4.0,
                base.y + f64::from((phase * 0.7).cos().abs()) * 2.0,
                base.z + f64::from(phase.cos()) * 4.0,
            );
            let _ = self.world.set(*id, "Position", Value::Vector3(pos));
        }
        gfx.renderer.sync(&self.world);
        self.stats.add_sync(sync_start.elapsed());
        match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                gfx.renderer.render(
                    &view,
                    &orbit_camera(t),
                    (gfx.config.width, gfx.config.height),
                );
                gfx.window.pre_present_notify();
                gfx.ctx.queue.present(frame);
                self.stats.frame();
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gfx.surface.configure(&gfx.ctx.device, &gfx.config);
            }
            _ => {}
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Metallic stress: 5,000 parts")
            .with_inner_size(PhysicalSize::new(1920, 1080));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let (ctx, surface) = RenderContext::new_for_window(window.clone()).expect("device");
        println!("adapter: {}", ctx.adapter_name);
        let adapter = ctx.adapter.as_ref().expect("adapter");
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(adapter, size.width.max(1), size.height.max(1))
            .expect("surface config");
        let caps = surface.get_capabilities(adapter);
        if let Some(f) = caps.formats.iter().find(|f| f.is_srgb()) {
            config.format = *f;
        }
        config.present_mode = if self.vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        surface.configure(&ctx.device, &config);
        let mut renderer = Renderer::with_settings(&ctx, config.format, self.settings);
        renderer.sync(&self.world);
        self.movers = collect_movers(&self.world);
        if !self.no_prewarm {
            let t = Instant::now();
            renderer.prewarm((config.width, config.height));
            println!("prewarm took {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "{} parts, {:?}, {}x{}, format {:?}, present {:?}",
            renderer.part_count(),
            self.settings.quality,
            config.width,
            config.height,
            config.format,
            config.present_mode
        );
        self.start = Instant::now();
        self.stats = Stats::new();
        self.gfx = Some(Gfx {
            window,
            surface,
            config,
            ctx,
            renderer,
            minimised: false,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.stats.summary();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.minimised = size.width == 0 || size.height == 0;
                    if !gfx.minimised {
                        gfx.config.width = size.width;
                        gfx.config.height = size.height;
                        gfx.surface.configure(&gfx.ctx.device, &gfx.config);
                    }
                }
            }
            WindowEvent::RedrawRequested => self.render(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(gfx) = self.gfx.as_ref() else {
            return;
        };
        let drawable = !gfx.minimised && gfx.config.width > 0 && gfx.config.height > 0;
        let deadline = self
            .seconds
            .and_then(|s| Duration::try_from_secs_f32(s).ok())
            .map(|d| self.start + d);
        if !drawable && deadline.is_some_and(|d| Instant::now() >= d) {
            self.stats.summary();
            event_loop.exit();
            return;
        }
        event_loop.set_control_flow(control_flow(drawable, deadline));
        if drawable {
            gfx.window.request_redraw();
        }
    }
}

/// How the event loop runs: flat out while there is something to draw; asleep until the next
/// event (a restore sends `Resized`) or the `--seconds` deadline while minimised, instead of
/// spinning at 100% CPU.
fn control_flow(drawable: bool, deadline: Option<Instant>) -> ControlFlow {
    match (drawable, deadline) {
        (true, _) => ControlFlow::Poll,
        (false, Some(d)) => ControlFlow::WaitUntil(d),
        (false, None) => ControlFlow::Wait,
    }
}

fn screenshot(path: &str, settings: RenderSettings) {
    let world = build_world();
    let ctx = RenderContext::new_headless().expect("headless device");
    println!("adapter: {}", ctx.adapter_name);
    let mut renderer = Renderer::with_settings(&ctx, wgpu::TextureFormat::Rgba8UnormSrgb, settings);
    renderer.sync(&world);
    let (w, h) = (1920, 1080);
    let cam = Camera {
        position: Vec3::new(120.0, 52.0, -125.0),
        look_at: Vec3::new(0.0, 4.0, 0.0),
        fov_deg: 70.0,
    };
    let pixels = renderer.capture(&cam, (w, h)).expect("capture");
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .expect("png header")
        .write_image_data(&pixels)
        .expect("png data");
    println!("wrote {path}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let settings = RenderSettings {
        quality: if flag("--low") {
            Quality::Low
        } else {
            Quality::High
        },
    };
    if let Some(path) = value("--screenshot") {
        screenshot(&path, settings);
        return;
    }
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        gfx: None,
        world: build_world(),
        movers: Vec::new(),
        no_prewarm: flag("--no-prewarm"),
        settings,
        vsync: flag("--vsync"),
        seconds: value("--seconds").and_then(|s| s.parse().ok()),
        stats: Stats::new(),
        start: Instant::now(),
    };
    event_loop.run_app(&mut app).expect("run");
}

// Run with `cargo test -p metallic_render --example stress`.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_times_stay_bounded_and_keep_the_worst() {
        let mut t = FrameTimes::new();
        for i in 0..(RECENT_FRAMES as u64 * 3) {
            t.push(if i == 7 { 120.0 } else { (i % 10) as f32 });
        }
        assert_eq!(t.count, RECENT_FRAMES as u64 * 3);
        assert_eq!(t.recent.len(), RECENT_FRAMES);
        assert!(t.recent.capacity() < RECENT_FRAMES * 2);
        assert_eq!(t.worst.len(), WORST_FRAMES);
        assert_eq!(t.worst[0], (7, 120.0));
        assert!(t.worst.windows(2).all(|w| w[0].1 >= w[1].1));
        assert_eq!(t.over_33ms, 1);
        assert_eq!(t.recent_quantile(0.0), 0.0);
        assert_eq!(t.recent_quantile(1.0), 9.0);
    }

    #[test]
    fn minimised_waits_instead_of_polling() {
        let d = Instant::now() + Duration::from_secs(3);
        assert_eq!(control_flow(true, None), ControlFlow::Poll);
        assert_eq!(control_flow(true, Some(d)), ControlFlow::Poll);
        assert_eq!(control_flow(false, None), ControlFlow::Wait);
        assert_eq!(control_flow(false, Some(d)), ControlFlow::WaitUntil(d));
    }
}
