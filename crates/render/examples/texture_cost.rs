//! What a new texture costs (#152): making it (on the CPU), its first draw against later ones, freeing it, and
//! whether a frame that writes a buffer waits while another thread makes one, on the backend `WGPU_BACKEND` picks.
//! The numbers behind [`metallic_render`]'s textures made off the frame, in `docs/perf.md`.
//!
//! ```text
//! cargo run --release -p metallic_render --example texture_cost
//! ```
//!
//! Each size is made twice in a row (a round each), so the second round shows what reusing freed memory costs.

use std::time::Instant;

use metallic_render::{ContextOptions, RenderContext, wgpu};

fn main() {
    let context = RenderContext::new_offscreen(&ContextOptions::default()).expect("a graphics adapter");
    let caps = context.capabilities();
    println!("{:?}, {}", caps.backend, caps.adapter_name);
    let device = context.device().clone();
    let small = depth(&device, 256, 1, 1);
    for _ in 0..5 {
        clear(&context, &[&small]);
    }
    // Name, side, layers, samples.
    let sizes: [(&str, u32, u32, u32); 7] = [
        ("2048² (16 MiB)", 2048, 1, 1),
        ("3072² (36 MiB)", 3072, 1, 1),
        ("4096² (64 MiB)", 4096, 1, 1),
        ("3072² x3 (108 MiB, Ultra's cascades)", 3072, 3, 1),
        ("1920² 8x MSAA (113 MiB)", 1920, 1, 8),
        ("6144² (144 MiB)", 6144, 1, 1),
        ("8192² (256 MiB)", 8192, 1, 1),
    ];
    for round in 1..=2 {
        for (name, side, layers, samples) in sizes {
            let start = Instant::now();
            let texture = depth(&device, side, layers, samples);
            let made = ms(start);
            let first = clear(&context, &[&texture]);
            let again = clear(&context, &[&texture]);
            println!("round {round}, {name}: made in {made:.2} ms, first clear {first:.2} ms, then {again:.2} ms");
            drop(texture);
            clear(&context, &[&small]);
        }
    }
    for (name, side, layers) in [("2048² x3", 2048, 3), ("3072² x3", 3072, 3), ("6144²", 6144, 1)] {
        let texture = depth(&device, side, layers, 1);
        clear(&context, &[&texture]);
        let start = Instant::now();
        drop(texture);
        // The GPU is idle, so wgpu frees it now.
        println!("freeing {name}: {:.2} ms", ms(start));
    }
    // A worker makes a large texture while this thread draws frames that write a buffer, as every frame does.
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("texture cost uniform"),
        size: 65536,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let data = vec![1u8; 65536];
    for writes in [true, false] {
        let worker_device = device.clone();
        let started = Instant::now();
        let worker = std::thread::spawn(move || {
            let start = Instant::now();
            let texture = depth(&worker_device, 6144, 1, 1);
            (texture, ms(start))
        });
        let (mut frames, mut worst) = (0, 0.0f64);
        while started.elapsed().as_millis() < 100 {
            let start = Instant::now();
            if writes {
                context.queue().write_buffer(&buffer, 0, &data);
            }
            clear(&context, &[&small]);
            worst = worst.max(ms(start));
            frames += 1;
        }
        let (texture, made) = worker.join().expect("the worker");
        println!(
            "a worker makes 6144² in {made:.2} ms; {frames} frames {} meanwhile, the slowest {worst:.2} ms",
            if writes { "writing a buffer" } else { "without writes" }
        );
        drop(texture);
    }
}

/// A `side`² depth texture of `layers` layers and `samples` samples, as the shadow maps and the scene's depth are.
fn depth(device: &wgpu::Device, side: u32, layers: u32, samples: u32) -> wgpu::Texture {
    let sampled = if samples == 1 {
        wgpu::TextureUsages::TEXTURE_BINDING
    } else {
        wgpu::TextureUsages::empty()
    };
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("texture cost"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | sampled,
        view_formats: &[],
    })
}

/// Clears every layer of `textures` in a submission of its own and waits for it: its wall time in milliseconds.
fn clear(context: &RenderContext, textures: &[&wgpu::Texture]) -> f64 {
    let device = context.device();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    for texture in textures {
        for layer in 0..texture.depth_or_array_layers() {
            let view = texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            });
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            }));
        }
    }
    let start = Instant::now();
    let submission = context.queue().submit([encoder.finish()]);
    // Outside any frame: an example may wait.
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    });
    ms(start)
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}
