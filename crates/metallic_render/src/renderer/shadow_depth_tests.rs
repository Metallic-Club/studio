//! Numeric GPU checks using the production vertex entries and cascade/depth helpers.
use super::*;
use glam::{Mat4, Vec3};
use metallic_asset::{MaterialSlot, Skeleton, SkinnedMesh, SkinnedModel, SkinnedVertex, Transform};

const SIZE: u32 = 16;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const PROBE: &str = r#"
@fragment
fn fs_shadow_depth_probe(in: VOut) -> @location(0) vec4<f32> {
    let depth = fragment_view_depth(in);
    // Equality is tested directly on a uniform split, independently of raster rounding.
    let boundary = shadow_cascade(g.cascade_splits[u32(g.params.y)]);
    return vec4<f32>(depth, f32(shadow_cascade(depth)), 1.0, f32(boundary));
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
        Err(error @ RenderError::NoAdapter(_)) => {
            eprintln!("shadow-depth probe UNAVAILABLE: {backends:?}: {error}");
            assert!(
                std::env::var_os("METALLIC_REQUIRE_SHADOW_PROBE").is_none(),
                "required shadow-depth adapter unavailable: {error}"
            );
            None
        }
        Err(error) => panic!("shadow-depth probe device setup failed: {error}"),
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

#[derive(Clone, Copy, Debug)]
enum Route {
    Part,
    Skinned,
}

fn pipeline(renderer: &Renderer, route: Route) -> wgpu::RenderPipeline {
    let mut parts = SCENE_PARTS.to_vec();
    parts.push(PROBE);
    let module = shader(&renderer.device, "shadow depth numeric probe", &parts);
    let layouts = match route {
        Route::Part => vec![Some(&renderer.global_layout)],
        Route::Skinned => vec![Some(&renderer.global_layout), Some(&renderer.skin_layout)],
    };
    let layout = renderer
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow depth numeric probe"),
            bind_group_layouts: &layouts,
            immediate_size: 0,
        });
    let buffers = match route {
        Route::Part => vertex_layouts().map(Some).to_vec(),
        Route::Skinned => vec![Some(skinned_layout())],
    };
    renderer
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow depth numeric probe"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(match route {
                    Route::Part => "vs_main",
                    Route::Skinned => "vs_skinned",
                }),
                compilation_options: Default::default(),
                buffers: &buffers,
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

fn renderer(ctx: &RenderContext) -> Renderer {
    Renderer::with_settings(
        ctx,
        wgpu::TextureFormat::Rgba8Unorm,
        RenderSettings {
            quality: Quality::Low,
        },
    )
}

fn constant_triangle(depth: f32) -> [[f32; 3]; 3] {
    [(-1.0, -1.0), (3.0, -1.0), (-1.0, 3.0)].map(|(x, y)| [x * depth, y * depth, -depth])
}

fn skinned_triangle(
    ctx: &RenderContext,
    renderer: &Renderer,
    positions: [[f32; 3]; 3],
) -> (GpuSkinnedModel, Vec<SkinSlot>) {
    // A nonidentity joint proves the real skin upload/vertex path is exercised.
    let offset = Vec3::new(3.0, -2.0, -7.0);
    let bind_positions = positions.map(|p| Vec3::from_array(p) - offset);
    let model = Arc::new(SkinnedModel {
        skeleton: Skeleton {
            names: vec!["probe".into()],
            parents: vec![None],
            rest: vec![Transform::IDENTITY],
            inverse_bind: vec![Mat4::IDENTITY],
            base: Mat4::IDENTITY,
        },
        lods: vec![SkinnedMesh {
            name: "probe".into(),
            vertices: bind_positions
                .map(|p| SkinnedVertex {
                    position: p.to_array(),
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0; 2],
                    joints: [0; 4],
                    weights: [1.0, 0.0, 0.0, 0.0],
                    material: 0,
                })
                .to_vec(),
            indices: vec![0, 1, 2],
            min: bind_positions
                .into_iter()
                .fold(Vec3::splat(f32::INFINITY), Vec3::min),
            max: bind_positions
                .into_iter()
                .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max),
        }],
        materials: vec![MaterialSlot {
            name: "probe".into(),
            base_color: [1.0; 4],
            roughness: 0.5,
            metallic: 0.0,
        }],
        clips: Vec::new(),
    });
    let gpu = GpuSkinnedModel::upload(&ctx.device, &model);
    let joints = [Mat4::from_translation(offset)];
    let instance = SkinnedInstance {
        model: &model,
        joints: &joints,
        slot_colors: &[],
        center: Vec3::ZERO,
        height: 1.0,
        detail_scale: 1.0,
        style: SkinStyle::Standard,
    };
    let mut slots = Vec::new();
    SkinSlot::write(
        &mut slots,
        0,
        &ctx.device,
        &ctx.queue,
        &renderer.skin_layout,
        &instance,
        &mut Vec::new(),
    );
    (gpu, slots)
}

fn render(
    ctx: &RenderContext,
    renderer: &Renderer,
    pipeline: &wgpu::RenderPipeline,
    route: Route,
    positions: [[f32; 3]; 3],
    boundary: usize,
    label: &str,
) -> Vec<[f32; 4]> {
    let mut uniforms = globals();
    uniforms.params[1] = boundary as f32;
    ctx.queue
        .write_buffer(&renderer.globals, 0, bytemuck::bytes_of(&uniforms));
    let vertices = positions.map(|position| Vertex {
        position,
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
    let skin = matches!(route, Route::Skinned).then(|| skinned_triangle(ctx, renderer, positions));
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &renderer.global_group, &[]);
        if let Some((gpu, slots)) = &skin {
            pass.set_bind_group(1, &slots[0].bind_group, &[]);
            pass.set_vertex_buffer(0, gpu.lods[0].vertices.slice(..));
            pass.set_index_buffer(gpu.lods[0].indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..gpu.lods[0].index_count, 0, 0..1);
        } else {
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_vertex_buffer(1, instance.slice(..));
            pass.draw(0..3, 0..1);
        }
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
    dump(ctx, label, &pixels);
    pixels
}

fn dump(ctx: &RenderContext, label: &str, pixels: &[[f32; 4]]) {
    let Some(directory) = std::env::var_os("METALLIC_SHADOW_PROBE_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).expect("external probe output directory");
    let info = ctx.adapter.as_ref().expect("headless adapter").get_info();
    let name = format!("{:?}-{label}", info.backend);
    let bytes: Vec<u8> = pixels
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    std::fs::write(directory.join(format!("{name}.rgba32f")), bytes)
        .expect("external numeric image");
    let metadata = serde_json::json!({"width":SIZE,"height":SIZE,"channels":["depth","cascade","coverage","split_equality"],"encoding":"little-endian f32","backend":format!("{:?}",info.backend),"adapter":info.name});
    std::fs::write(directory.join(format!("{name}.json")), metadata.to_string())
        .expect("external numeric image metadata");
}

fn render_parts(ctx: &RenderContext, depth: f32) -> Vec<[f32; 4]> {
    let renderer = renderer(ctx);
    let pipeline = pipeline(&renderer, Route::Part);
    render(
        ctx,
        &renderer,
        &pipeline,
        Route::Part,
        constant_triangle(depth),
        0,
        "initial-part",
    )
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

fn observe(
    failures: &mut Vec<String>,
    label: &str,
    actual: [f32; 4],
    depth: f32,
    cascade: usize,
    boundary: usize,
) {
    eprintln!(
        "shadow-depth {label}: expected depth={depth}, cascade={cascade}, equality={boundary}; actual={actual:?}"
    );
    if actual[2] != 1.0
        || !actual[0].is_finite()
        || (actual[0] - depth).abs() > depth * 1e-4
        || actual[1] != cascade as f32
        || actual[3] != boundary as f32
    {
        failures.push(format!(
            "{label}: {actual:?}; expected depth={depth}, cascade={cascade}, equality={boundary}"
        ));
    }
}

fn matrix(backends: wgpu::Backends) {
    let Some(ctx) = context(backends) else {
        return;
    };
    let renderer = renderer(&ctx);
    let splits = cascade_splits(NEAR, SHADOW_DISTANCE, MAX_CASCADES);
    let mut failures = Vec::new();
    for route in [Route::Part, Route::Skinned] {
        let pipeline = pipeline(&renderer, route);
        for (i, split) in splits.iter().copied().enumerate() {
            // The after-last case expects the count sentinel, not the last cascade.
            for (side, depth, expected) in
                [("before", split * 0.98, i), ("after", split * 1.02, i + 1)]
            {
                let label = format!("{route:?}-split-{i}-{side}");
                let pixels = render(
                    &ctx,
                    &renderer,
                    &pipeline,
                    route,
                    constant_triangle(depth),
                    i,
                    &label,
                );
                for (x, y) in [(4, 4), (8, 8), (11, 11)] {
                    observe(
                        &mut failures,
                        &format!("{label}-pixel-{x}-{y}"),
                        pixels[(y * SIZE + x) as usize],
                        depth,
                        expected,
                        i + 1,
                    );
                }
            }
        }
        // NDC vertices (-1,-1), (1,-1), (0,1), with unequal clip-w. At the
        // sampled pixel centres the correct depth is the reciprocal-weighted
        // plane value, far from either a flat vertex value or a screen-linear sum.
        let depths = [8.0, 64.0, 256.0];
        let positions = [
            [-depths[0], -depths[0], -depths[0]],
            [depths[1], -depths[1], -depths[1]],
            [0.0, depths[2], -depths[2]],
        ];
        let label = format!("{route:?}-perspective-slope");
        let pixels = render(&ctx, &renderer, &pipeline, route, positions, 2, &label);
        for (x, y) in [(4, 10), (8, 10), (12, 11), (7, 5)] {
            // Readback rows start at the top, while projected NDC y points up.
            let ndc_x = 2.0 * (x as f32 + 0.5) / SIZE as f32 - 1.0;
            let ndc_y = 1.0 - 2.0 * (y as f32 + 0.5) / SIZE as f32;
            let c = (ndc_y + 1.0) * 0.5;
            let b = (ndc_x + 1.0 - c) * 0.5;
            let a = 1.0 - b - c;
            assert!(
                a > 0.0 && b > 0.0 && c > 0.0,
                "sample is inside the triangle"
            );
            let depth = 1.0 / (a / depths[0] + b / depths[1] + c / depths[2]);
            let screen_linear = a * depths[0] + b * depths[1] + c * depths[2];
            assert!(
                (screen_linear - depth).abs() > depth * 0.5,
                "fixture must distinguish linear interpolation"
            );
            assert!(
                depths.iter().all(|d| (d - depth).abs() > depth * 0.1),
                "fixture must distinguish flat interpolation"
            );
            let cascade = splits.partition_point(|split| depth >= *split);
            eprintln!("shadow-depth {label}-pixel-{x}-{y}: screen-linear control={screen_linear}");
            observe(
                &mut failures,
                &format!("{label}-pixel-{x}-{y}"),
                pixels[(y * SIZE + x) as usize],
                depth,
                cascade,
                MAX_CASCADES,
            );
        }
    }
    assert!(
        failures.is_empty(),
        "{} numeric shadow-depth observations failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn dx12_production_part_and_skin_depth_matrix() {
    matrix(wgpu::Backends::DX12);
}

#[test]
fn vulkan_production_part_and_skin_depth_matrix() {
    matrix(wgpu::Backends::VULKAN);
}

#[test]
fn production_depth_probe_shader_validates() {
    let mut parts = SCENE_PARTS.to_vec();
    parts.push(PROBE);
    super::tests::validate_wgsl("production shadow-depth probe", &parts);
}
