//! Stress test: 5,000 random Parts on a baseplate in a 1920x1080 window.
//!
//! Prints average and minimum fps every 5 seconds.
//!
//! Options: `--seconds N` exit after N seconds and print the overall average,
//! `--vsync` present with vsync (default is uncapped so the number means throughput),
//! `--low` use the Low quality preset,
//! `--screenshot PATH` render one frame offscreen (no window) to a PNG and exit.
use metallic_core::{Color3, Value, Vec3 as V3, World};
use metallic_render::glam::Vec3;
use metallic_render::{Camera, Quality, RenderContext, RenderSettings, Renderer};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

const PART_COUNT: usize = 5000;

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

fn orbit_camera(t: f32) -> Camera {
    let a = t * 0.15;
    Camera {
        position: Vec3::new(130.0 * a.cos(), 55.0, 130.0 * a.sin()),
        look_at: Vec3::new(0.0, 4.0, 0.0),
        fov_deg: 70.0,
    }
}

struct Stats {
    window_start: Instant,
    frames: u32,
    worst: Duration,
    last_frame: Instant,
    total_start: Instant,
    total_frames: u64,
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
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        self.worst = self.worst.max(now - self.last_frame);
        self.last_frame = now;
        self.frames += 1;
        self.total_frames += 1;
        let elapsed = now - self.window_start;
        if elapsed >= Duration::from_secs(5) {
            println!(
                "fps avg {:.1}  min {:.1}  ({} frames in {:.1}s)",
                f64::from(self.frames) / elapsed.as_secs_f64(),
                1.0 / self.worst.as_secs_f64(),
                self.frames,
                elapsed.as_secs_f64()
            );
            self.window_start = now;
            self.frames = 0;
            self.worst = Duration::ZERO;
        }
    }

    fn overall(&self) -> f64 {
        self.total_frames as f64 / self.total_start.elapsed().as_secs_f64()
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
            println!("overall fps avg {:.1} over {:.1}s", self.stats.overall(), t);
            event_loop.exit();
            return;
        }
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
                println!("overall fps avg {:.1}", self.stats.overall());
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

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(gfx) = self.gfx.as_ref() {
            gfx.window.request_redraw();
        }
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
        settings,
        vsync: flag("--vsync"),
        seconds: value("--seconds").and_then(|s| s.parse().ok()),
        stats: Stats::new(),
        start: Instant::now(),
    };
    event_loop.run_app(&mut app).expect("run");
}
