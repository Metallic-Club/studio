//! Numeric GPU checks using the production vertex entries and cascade/depth helpers.
use super::*;
use glam::{Mat4, Vec3};

const SIZE: u32 = 16;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const PROBE: &str = r#"
@fragment
fn fs_shadow_depth_probe(in: VOut) -> @location(0) vec4<f32> {
    let depth = fragment_view_depth(in);
    return vec4<f32>(depth, f32(shadow_cascade(depth)), 1.0, 1.0);
}
"#;

fn context(backends: wgpu::Backends) -> Option<RenderContext> {
    match RenderContext::new_headless_with_backends(backends) {
        Ok(ctx) => {
            let info = ctx.adapter.as_ref().expect("headless adapter").get_info();
            eprintln!(
                "shadow-depth probe EXECUTED: {:?}, {}",
                info.backend, info.name
            );
            Some(ctx)
        }
        Err(error) => {
            eprintln!("shadow-depth probe UNAVAILABLE: {backends:?}: {error}");
            assert!(
                std::env::var_os("METALLIC_REQUIRE_SHADOW_PROBE").is_none(),
                "required shadow-depth adapter unavailable: {error}"
            );
            None
        }
    }
}

fn globals() -> Globals {
    let camera = Camera {
        position: Vec3::ZERO,
        look_at: -Vec3::Z,
        fov_deg: 90.0,
    };
    let mut globals = Globals::zeroed();
    globals.view_proj = camera.view_proj(1.0).to_cols_array_2d();
    globals.params[0] = MAX_CASCADES as f32;
    let splits = cascade_splits(NEAR, SHADOW_DISTANCE, MAX_CASCADES);
    globals.cascade_splits[..MAX_CASCADES].copy_from_slice(&splits);
    globals
}

fn pipeline(renderer: &Renderer) -> wgpu::RenderPipeline {
    let mut parts = SCENE_PARTS.to_vec();
    parts.push(PROBE);
    let module = shader(&renderer.device, "shadow depth numeric probe", &parts);
    let layout = renderer
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow depth numeric probe"),
            bind_group_layouts: &[Some(&renderer.global_layout)],
            immediate_size: 0,
        });
    renderer
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow depth numeric probe"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &vertex_layouts().map(Some),
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_shadow_depth_probe"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
}

fn render_parts(ctx: &RenderContext, depth: f32) -> Vec<[f32; 4]> {
    let renderer = Renderer::with_settings(
        ctx,
        wgpu::TextureFormat::Rgba8Unorm,
        RenderSettings {
            quality: Quality::Low,
        },
    );
    ctx.queue
        .write_buffer(&renderer.globals, 0, bytemuck::bytes_of(&globals()));
    let pipeline = pipeline(&renderer);
    // All interior pixels are covered, with constant view depth.
    let vertices = [(-1.0, -1.0), (3.0, -1.0), (-1.0, 3.0)].map(|(x, y)| Vertex {
        position: [x * depth, y * depth, -depth],
        normal: [0.0, 0.0, 1.0],
    });
    let vertices = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow depth production vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let instance = InstanceRaw {
        model: Mat4::IDENTITY.to_cols_array_2d(),
        color: [1.0; 4],
        material: [0; 4],
    };
    let instance = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow depth production instance"),
            contents: bytemuck::bytes_of(&instance),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow depth numeric target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let readback = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shadow depth numeric readback"),
        size: u64::from(SIZE * SIZE * 16),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow depth numeric probe"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &renderer.global_group, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_vertex_buffer(1, instance.slice(..));
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 16),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    ctx.queue.submit(Some(encoder.finish()));
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
    ctx.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("probe GPU completion");
    rx.recv()
        .expect("probe map callback")
        .expect("probe buffer mapping");
    let mapped = readback
        .slice(..)
        .get_mapped_range()
        .expect("probe mapped bytes");
    let pixels = bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec();
    drop(mapped);
    readback.unmap();
    pixels
}

#[test]
fn dx12_production_part_constant_depth() {
    let Some(ctx) = context(wgpu::Backends::DX12) else {
        return;
    };
    let depth = globals().cascade_splits[0] * 1.25;
    let pixels = render_parts(&ctx, depth);
    let pixel = pixels[(SIZE * (SIZE / 2) + SIZE / 2) as usize];
    eprintln!("shadow-depth constant: expected depth={depth}, cascade=1; actual={pixel:?}");
    assert_eq!(pixel[2], 1.0, "production triangle must cover the sample");
    assert!(
        (pixel[0] - depth).abs() < depth * 1e-4,
        "view depth: {pixel:?}, expected {depth}"
    );
    assert_eq!(pixel[1], 1.0, "cascade selection beyond the first split");
}
