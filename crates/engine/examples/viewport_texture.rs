//! A game drawn into a viewport texture and shown in a window the example owns: the seam Studio's viewport panel uses
//! (ADR 0009, #213), with a plain textured quad standing in for egui-wgpu.
//!
//! ```text
//! cargo run -p metallic_engine --example viewport_texture            # until the window is closed
//! cargo run -p metallic_engine --example viewport_texture -- 300     # or for 300 frames
//! ```
//!
//! The example makes the window (with `winit`, a dev-dependency of the examples only) and the device for it
//! ([`RenderContext::new_with_window`]), as Studio does. A [`SceneRenderer`] attached to that device draws the game
//! into a [`ViewportTarget`] sized to a "panel" two thirds of the window; the example then clears the window's surface
//! to a UI grey and draws the target's texture into the panel's rectangle, as egui-wgpu draws a registered native
//! texture, and presents. Resize the window: the panel follows, and the target switches to its new size without a
//! black frame (until it does, the old texture is stretched over the panel). Each switch is printed, with the frame
//! rate once a second.

use std::sync::Arc;
use std::time::{Duration, Instant};

use metallic_engine::metallic_core::glam::Vec3;
use metallic_engine::metallic_core::{CFrame, Color3, DataModel, EnumItem, Id, Value};
use metallic_engine::metallic_render::settings::{GraphicsSettings, Preset};
use metallic_engine::metallic_render::{ContextOptions, RenderContext, RenderSurface, ViewportTarget, wgpu};
use metallic_engine::{FrameStats, SceneRenderer};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

/// Draws a texture over the whole viewport the pass sets: two triangles from the vertex index, no buffers.
const QUAD: &str = r"
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) index: u32) -> Out {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let uv = corners[index];
    var out: Out;
    out.position = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, in.uv);
}
";

/// The window's background around the panel (linear).
const UI_GREY: wgpu::Color = wgpu::Color {
    r: 0.012,
    g: 0.011,
    b: 0.016,
    a: 1.0,
};

/// The UI side: what egui-wgpu would hold for one native texture.
struct QuadPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The bind group of the target's current view, and the generation it was made for.
    group: Option<(u64, wgpu::BindGroup)>,
}

impl QuadPass {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport quad"),
            source: wgpu::ShaderSource::Wgsl(QUAD.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport quad"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport quad"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport quad"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("viewport quad"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            group: None,
        }
    }

    /// Binds the target's view again when a resize replaced it (egui-wgpu: `update_egui_texture_from_wgpu_texture`).
    fn update(&mut self, device: &wgpu::Device, target: &ViewportTarget) {
        if self
            .group
            .as_ref()
            .is_some_and(|(generation, _)| *generation == target.generation())
        {
            return;
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport quad"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(target.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let (width, height) = target.size();
        println!("viewport texture {width}x{height} (generation {})", target.generation());
        self.group = Some((target.generation(), group));
    }

    /// Clears `view` to the UI grey and draws the texture over `panel` (x, y, width, height in pixels).
    fn draw(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, panel: [f32; 4]) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(UI_GREY),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some((_, group)) = self.group.as_ref() {
            let [x, y, width, height] = panel;
            pass.set_viewport(x, y, width, height, 0.0, 1.0);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}

/// The panel in a `width` x `height` window: two thirds of it, centred, in whole pixels.
fn panel((width, height): (u32, u32)) -> (u32, u32, u32, u32) {
    let (w, h) = ((width * 2 / 3).max(1), (height * 2 / 3).max(1));
    ((width - w) / 2, (height - h) / 2, w, h)
}

/// A small game: a floor, a few shapes, and the camera that circles them.
fn game() -> (DataModel, Id) {
    let mut dm = metallic_classes::new_game().expect("a game");
    let registry = dm.registry().clone();
    let workspace = dm
        .service(registry.class_id("Workspace").expect("Workspace"))
        .expect("a Workspace");
    let part_class = registry.class_id("Part").expect("Part");
    let mut part = |shape: &str, material: &str, size: Vec3, at: Vec3, rgb: [u8; 3]| {
        let id = dm.create(part_class).expect("a part");
        dm.set_by_name(id, "Shape", Value::Enum(EnumItem::new("PartShape", shape)))
            .expect("Shape");
        dm.set_by_name(id, "Material", Value::Enum(EnumItem::new("Material", material)))
            .expect("Material");
        dm.set_by_name(id, "Size", Value::Vector3(size)).expect("Size");
        dm.set_by_name(id, "CFrame", Value::CFrame(CFrame::new(at)))
            .expect("CFrame");
        let [r, g, b] = rgb;
        dm.set_by_name(id, "Color", Value::Color3(Color3::from_rgb_u8(r, g, b)))
            .expect("Color");
        dm.set_parent(id, Some(workspace)).expect("parented");
    };
    part(
        "Block",
        "Concrete",
        Vec3::new(40.0, 1.0, 40.0),
        Vec3::new(0.0, -0.5, 0.0),
        [128, 126, 120],
    );
    part(
        "Block",
        "Plastic",
        Vec3::splat(3.0),
        Vec3::new(-6.0, 1.5, -1.0),
        [196, 40, 28],
    );
    part(
        "Ball",
        "SmoothPlastic",
        Vec3::splat(3.5),
        Vec3::new(-1.5, 1.75, -3.0),
        [13, 105, 172],
    );
    part(
        "Cylinder",
        "Wood",
        Vec3::new(5.0, 2.0, 2.0),
        Vec3::new(3.0, 1.0, -1.0),
        [160, 95, 53],
    );
    part(
        "Wedge",
        "Grass",
        Vec3::new(3.0, 2.5, 4.0),
        Vec3::new(7.0, 1.25, -2.0),
        [75, 151, 75],
    );
    part(
        "Ball",
        "Metal",
        Vec3::splat(2.0),
        Vec3::new(0.5, 1.0, 2.0),
        [226, 155, 64],
    );
    part(
        "Block",
        "Neon",
        Vec3::splat(1.0),
        Vec3::new(-3.0, 0.5, 3.0),
        [0, 255, 191],
    );
    let Ok(Value::Ref(Some(camera))) = dm.get_by_name(workspace, "CurrentCamera") else {
        panic!("a new game has a CurrentCamera");
    };
    dm.set_by_name(camera, "FieldOfView", Value::Number(60.0))
        .expect("FieldOfView");
    (dm, camera)
}

struct Running {
    window: Arc<Window>,
    context: RenderContext,
    surface: RenderSurface,
    scene: SceneRenderer,
    quad: QuadPass,
    dm: DataModel,
    camera: Id,
}

struct App {
    running: Option<Running>,
    frame_limit: Option<u64>,
    started: Instant,
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
            .with_title("metallic_engine: viewport texture")
            .with_inner_size(LogicalSize::new(1080.0, 640.0));
        let window = Arc::new(event_loop.create_window(attributes).expect("a window"));
        let size = window.inner_size();
        // The caller's window and the device for it; the UI side draws with the same device and queue.
        let (context, surface) =
            RenderContext::new_with_window(Arc::clone(&window), size.width, size.height, &ContextOptions::default())
                .expect("a device that can present to the window");
        let caps = context.capabilities();
        println!(
            "adapter: {} ({:?}, {:?}); surface {:?}",
            caps.adapter_name,
            caps.backend,
            caps.device_type,
            surface.format()
        );
        let quad = QuadPass::new(context.device(), surface.format());
        let (_, _, width, height) = panel((size.width, size.height));
        let mut scene = SceneRenderer::new(ContextOptions::default());
        scene.set_graphics(GraphicsSettings::new(Preset::High));
        scene.set_overlay(true);
        scene
            .attach_viewport(&context, width, height)
            .expect("a viewport target");
        let (dm, camera) = game();
        window.request_redraw();
        self.running = Some(Running {
            window,
            context,
            surface,
            scene,
            quad,
            dm,
            camera,
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
                // The panel follows the window; a minimised window keeps the panel's size.
                if size.width > 0 && size.height > 0 {
                    let (_, _, width, height) = panel((size.width, size.height));
                    running.scene.resize_viewport(width, height);
                }
            }
            WindowEvent::RedrawRequested => {
                let angle = self.started.elapsed().as_secs_f32() * 0.4;
                let eye = Vec3::new(angle.sin() * 16.0, 7.0, angle.cos() * 16.0);
                running
                    .dm
                    .set_by_name(
                        running.camera,
                        "CFrame",
                        Value::CFrame(CFrame::look_at(eye, Vec3::new(0.0, 1.0, 0.0), Vec3::Y)),
                    )
                    .expect("camera CFrame");
                // The game into the viewport texture (its own submission), then the UI over the window.
                running.scene.render_viewport(&mut running.dm, FrameStats::default());
                if let Some(target) = running.scene.viewport() {
                    running.quad.update(running.context.device(), target);
                }
                let image = running.surface.acquire(&running.context).expect("a surface image");
                if let Some(image) = image {
                    let (x, y, width, height) = panel(running.surface.size());
                    let mut encoder = running
                        .context
                        .device()
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui") });
                    running.quad.draw(
                        &mut encoder,
                        image.view(),
                        [x as f32, y as f32, width as f32, height as f32],
                    );
                    running.context.submit(Some(encoder.finish()));
                    running.window.pre_present_notify();
                    image.present(&running.context);
                    self.presented += 1;
                    self.frames_since_report += 1;
                }
                if self.last_report.elapsed() >= Duration::from_secs(1) {
                    let fps = f64::from(self.frames_since_report) / self.last_report.elapsed().as_secs_f64();
                    let size = running.scene.viewport().map(ViewportTarget::size);
                    println!("{fps:.0} fps, viewport {size:?}");
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
        started: Instant::now(),
        presented: 0,
        last_report: Instant::now(),
        frames_since_report: 0,
    };
    event_loop.run_app(&mut app).expect("the event loop runs");
}
