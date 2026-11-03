//! Windowed clear: opens a window (with `winit`, a dev-dependency of this example only; the engine's windows come
//! from `metallic_platform`), renders the clear colour every frame with vsync, and prints the per-pass GPU timings
//! once a second. Resize it, minimise it: the surface handles both.
//!
//! ```text
//! cargo run -p metallic_render --example window [frames]
//! ```
//!
//! With a frame count it closes by itself after that many frames.

use std::sync::Arc;
use std::time::{Duration, Instant};

use metallic_render::{ContextOptions, RenderContext, RenderSurface, Renderer, RendererSettings, wgpu};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.1,
    g: 0.3,
    b: 0.6,
    a: 1.0,
};

struct Running {
    window: Arc<Window>,
    context: RenderContext,
    surface: RenderSurface,
    renderer: Renderer,
}

struct App {
    running: Option<Running>,
    frame_limit: Option<u64>,
    presented: u64,
    last_report: Instant,
    frames_since_report: u32,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("metallic_render: clear")
            .with_inner_size(LogicalSize::new(960.0, 540.0));
        let window = Arc::new(event_loop.create_window(attributes).expect("a window"));
        let size = window.inner_size();
        let (context, surface) =
            RenderContext::new_with_window(Arc::clone(&window), size.width, size.height, &ContextOptions::default())
                .expect("a device that can present to the window");
        let caps = context.capabilities();
        println!(
            "adapter: {} ({:?}, {:?})",
            caps.adapter_name, caps.backend, caps.device_type
        );
        println!(
            "surface: {:?}, {:?}, {}x{}",
            surface.format(),
            surface.present_mode(),
            size.width,
            size.height
        );
        let renderer = Renderer::new(
            &context,
            RendererSettings {
                clear_color: CLEAR,
                ..RendererSettings::new(surface.format(), size.width.max(1), size.height.max(1))
            },
        )
        .expect("a renderer");
        window.request_redraw();
        self.running = Some(Running {
            window,
            context,
            surface,
            renderer,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(running) = self.running.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                running.surface.resize(&running.context, size.width, size.height);
                // A minimised window has no size; the renderer keeps its old one until it comes back.
                if size.width > 0 && size.height > 0 {
                    running
                        .renderer
                        .resize(size.width, size.height)
                        .expect("a size the device supports");
                }
            }
            WindowEvent::RedrawRequested => {
                let frame = running.surface.acquire(&running.context).expect("a surface image");
                if let Some(frame) = frame {
                    running.renderer.render(frame.view());
                    running.window.pre_present_notify();
                    frame.present(&running.context);
                    self.presented += 1;
                    self.frames_since_report += 1;
                }
                if self.last_report.elapsed() >= Duration::from_secs(1) {
                    let seconds = self.last_report.elapsed().as_secs_f64();
                    let fps = f64::from(self.frames_since_report) / seconds;
                    match running.renderer.gpu_timings() {
                        Some(timings) => println!(
                            "{fps:.0} fps, GPU {:.4} ms (frame {}): {}",
                            timings.total_ms(),
                            timings.frame(),
                            timings
                                .passes()
                                .iter()
                                .map(|pass| format!("{} {:.4}", pass.pass, pass.gpu_ms))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        None => println!("{fps:.0} fps, no GPU timings"),
                    }
                    self.last_report = Instant::now();
                    self.frames_since_report = 0;
                }
                if self.frame_limit.is_some_and(|limit| self.presented >= limit) {
                    println!("presented {} frames", self.presented);
                    event_loop.exit();
                    return;
                }
                running.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() {
    let frame_limit = std::env::args()
        .nth(1)
        .map(|arg| arg.parse().expect("the frame count is a number"));
    let event_loop = EventLoop::new().expect("an event loop");
    let mut app = App {
        running: None,
        frame_limit,
        presented: 0,
        last_report: Instant::now(),
        frames_since_report: 0,
    };
    event_loop.run_app(&mut app).expect("the event loop runs");
}
