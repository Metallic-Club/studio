//! Builds every style-shader sample as a real wgpu pipeline on the chosen backend and reports errors.
//!
//! Usage: `style_shaders_check <dx12|vulkan>`. Exit code 0 only if every module and pipeline was created
//! without a validation error and the particle simulation ran correctly. Pipelines use automatic layouts, so
//! this also checks that the shaders' resource bindings are consistent with what wgpu derives from them.
//!
//! The render samples are only built here (`render_looks` runs `surface`, `outline_post`, `tonemap` and
//! `pixel_art`). The particle compute passes are also run: `run_particles` simulates several frames on the
//! GPU and checks the read-back counters, args, lists and particles against a CPU model.

use std::borrow::Cow;
use std::process::ExitCode;

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const NORMALS: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const LDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SWAP: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn shader_source(name: &str) -> String {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    std::fs::read_to_string(format!("{dir}/{name}.wgsl")).unwrap_or_default()
}

struct Vb {
    stride: u64,
    step: wgpu::VertexStepMode,
    attrs: Vec<wgpu::VertexAttribute>,
}

fn vb(step: wgpu::VertexStepMode, formats: &[(u32, wgpu::VertexFormat)]) -> Vb {
    let mut offset = 0;
    let mut attrs = Vec::new();
    for (loc, f) in formats {
        attrs.push(wgpu::VertexAttribute { format: *f, offset, shader_location: *loc });
        offset += f.size();
    }
    Vb { stride: offset, step, attrs }
}

struct RenderCase<'a> {
    file: &'a str,
    vs: &'a str,
    fs: &'a str,
    buffers: Vec<Vb>,
    targets: Vec<(wgpu::TextureFormat, Option<wgpu::BlendState>)>,
    depth: bool,
    cull: Option<wgpu::Face>,
}

fn main() -> ExitCode {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "dx12".into());
    let backends = match arg.as_str() {
        "vulkan" => wgpu::Backends::VULKAN,
        _ => wgpu::Backends::DX12,
    };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle_from_env()
    });
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    })) else {
        eprintln!("no {arg} adapter");
        return ExitCode::FAILURE;
    };
    let info = adapter.get_info();
    println!("adapter: {} ({:?}, driver {})", info.name, info.backend, info.driver_info);
    let Ok((device, queue)) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
    else {
        eprintln!("device creation failed");
        return ExitCode::FAILURE;
    };

    use wgpu::VertexFormat as F;
    use wgpu::VertexStepMode as S;
    let premul = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
    let cases = vec![
        RenderCase {
            file: "surface",
            vs: "vs_main",
            fs: "fs_main",
            buffers: vec![
                vb(S::Vertex, &[(0, F::Float32x3), (1, F::Float32x3)]),
                vb(S::Instance, &[(2, F::Float32x4), (3, F::Float32x4), (4, F::Float32x4),
                    (5, F::Float32x4), (6, F::Float32x4), (7, F::Float32x4)]),
            ],
            targets: vec![(HDR, None), (NORMALS, None)],
            depth: true,
            cull: Some(wgpu::Face::Back),
        },
        RenderCase {
            file: "toon_ramp",
            vs: "vs_main",
            fs: "fs_main",
            buffers: vec![vb(S::Vertex, &[(0, F::Float32x3), (1, F::Float32x3), (2, F::Float32x2)])],
            targets: vec![(HDR, None)],
            depth: true,
            cull: Some(wgpu::Face::Back),
        },
        RenderCase {
            file: "outline_hull",
            vs: "vs_main",
            fs: "fs_main",
            buffers: vec![vb(S::Vertex, &[(0, F::Float32x3), (1, F::Float32x3), (2, F::Float32x3),
                (3, F::Float32x4)])],
            targets: vec![(HDR, None)],
            depth: true,
            cull: Some(wgpu::Face::Front),
        },
        RenderCase { file: "outline_post", vs: "vs_fullscreen", fs: "fs_main", buffers: vec![],
            targets: vec![(HDR, None)], depth: false, cull: None },
        RenderCase { file: "tonemap", vs: "vs_fullscreen", fs: "fs_main", buffers: vec![],
            targets: vec![(SWAP, None)], depth: false, cull: None },
        RenderCase { file: "pixel_art", vs: "vs_fullscreen", fs: "fs_quantize", buffers: vec![],
            targets: vec![(LDR, None)], depth: false, cull: None },
        RenderCase { file: "pixel_art", vs: "vs_fullscreen", fs: "fs_upscale", buffers: vec![],
            targets: vec![(SWAP, None)], depth: false, cull: None },
        RenderCase {
            file: "ps1_retro",
            vs: "vs_main",
            fs: "fs_main",
            buffers: vec![vb(S::Vertex, &[(0, F::Float32x3), (1, F::Float32x3), (2, F::Float32x2),
                (3, F::Float32x4)])],
            targets: vec![(HDR, None)],
            depth: true,
            cull: Some(wgpu::Face::Back),
        },
        RenderCase { file: "particles_draw", vs: "vs_main", fs: "fs_main", buffers: vec![],
            targets: vec![(HDR, premul)], depth: false, cull: None },
    ];

    let mut failures = 0;
    for case in &cases {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(case.file),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(shader_source(case.file))),
        });
        let layouts: Vec<wgpu::VertexBufferLayout> = case
            .buffers
            .iter()
            .map(|b| wgpu::VertexBufferLayout { array_stride: b.stride, step_mode: b.step, attributes: &b.attrs })
            .collect();
        let targets: Vec<Option<wgpu::ColorTargetState>> = case
            .targets
            .iter()
            .map(|(format, blend)| Some(wgpu::ColorTargetState { format: *format, blend: *blend,
                write_mask: wgpu::ColorWrites::ALL }))
            .collect();
        let _pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(case.file),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(case.vs),
                compilation_options: Default::default(),
                buffers: &layouts.iter().cloned().map(Some).collect::<Vec<_>>(),
            },
            primitive: wgpu::PrimitiveState { cull_mode: case.cull, ..Default::default() },
            depth_stencil: case.depth.then(|| wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(case.fs),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        match pollster::block_on(scope.pop()) {
            None => println!("ok    render  {}::{} + {}", case.file, case.vs, case.fs),
            Some(e) => {
                failures += 1;
                println!("FAIL  render  {}::{} + {}: {e}", case.file, case.vs, case.fs);
            }
        }
    }

    for entry in ["cs_emit", "cs_prepare", "cs_update", "cs_finish"] {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles_sim"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(shader_source("particles_sim"))),
        });
        let _pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });
        match pollster::block_on(scope.pop()) {
            None => println!("ok    compute particles_sim::{entry}"),
            Some(e) => {
                failures += 1;
                println!("FAIL  compute particles_sim::{entry}: {e}");
            }
        }
    }

    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let sim = run_particles(&device, &queue);
    let sim = match (sim, pollster::block_on(scope.pop())) {
        (Ok(_), Some(e)) => Err(e.to_string()),
        (Err(m), Some(e)) => Err(format!("{m} (validation: {e})")),
        (r, None) => r,
    };
    match sim {
        Ok(summary) => println!("ok    run     particles_sim: {summary}"),
        Err(e) => {
            failures += 1;
            println!("FAIL  run     particles_sim: {e}");
        }
    }

    println!("{failures} failure(s)");
    if failures == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

const CAPACITY: u32 = 256;
const EMIT: u32 = 100;
const LIFETIME: f32 = 0.25;
const DT: f32 = 0.1;
const FRAMES: u32 = 8;

fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> Result<Vec<u32>, String> {
    let size = buffer.size();
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("readback") });
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
    queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("poll: {e}"))?;
    rx.recv().map_err(|e| e.to_string())?.map_err(|e| format!("map: {e}"))?;
    let out = {
        let mapped = slice.get_mapped_range().map_err(|e| format!("mapped range: {e}"))?;
        mapped.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
    };
    staging.unmap();
    Ok(out)
}

/// Runs `particles_sim.wgsl` for [`FRAMES`] frames (emit, prepare, indirect update, finish) and checks the
/// results after each frame against a CPU model: emission takes `min(EMIT, dead)` slots, a particle dies on
/// the update where its age reaches [`LIFETIME`], the alive and dead lists always hold every slot exactly once,
/// and the indirect args match the counts.
fn run_particles(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<String, String> {
    use wgpu::util::DeviceExt;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("particles_sim"),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(shader_source("particles_sim"))),
    });
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let sim_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("particles sim"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            storage(1),
            storage(2),
            storage(3),
        ],
    });
    let args_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("particles args"),
        entries: &[storage(0)],
    });
    // cs_emit and cs_update see group 0 only, so cs_update can read its indirect args from the args buffer.
    let sim_only = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("particles sim"),
        bind_group_layouts: &[Some(&sim_layout)],
        immediate_size: 0,
    });
    let with_args = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("particles sim + args"),
        bind_group_layouts: &[Some(&sim_layout), Some(&args_layout)],
        immediate_size: 0,
    });
    let pipeline = |entry: &str, layout: &wgpu::PipelineLayout| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let emit = pipeline("cs_emit", &sim_only);
    let prepare = pipeline("cs_prepare", &with_args);
    let update = pipeline("cs_update", &sim_only);
    let finish = pipeline("cs_finish", &with_args);

    let rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("emitter"),
        size: 128,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let particles = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("particles"),
        size: u64::from(CAPACITY) * 64,
        usage: rw,
        mapped_at_creation: false,
    });
    // Alive A and B empty, the dead list holding every slot.
    let mut init = vec![0u32; 3 * CAPACITY as usize];
    for i in 0..CAPACITY {
        init[(2 * CAPACITY + i) as usize] = i;
    }
    let lists = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("lists"),
        contents: &words(&init),
        usage: rw,
    });
    let counters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("counters"),
        contents: &words(&[0, 0, CAPACITY, 0]),
        usage: rw,
    });
    let args = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("args"),
        size: 32,
        usage: rw | wgpu::BufferUsages::INDIRECT,
        mapped_at_creation: false,
    });
    let sim_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("particles sim"),
        layout: &sim_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: particles.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: lists.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: counters.as_entire_binding() },
        ],
    });
    let args_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("particles args"),
        layout: &args_layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: args.as_entire_binding() }],
    });

    // CPU model: batches of (count, updates survived so far).
    let mut batches: Vec<(u32, u32)> = Vec::new();
    let mut dead = CAPACITY;
    let mut parity = 0u32;
    let mut emitted_total = 0;
    let mut died_total = 0;
    for frame in 0..FRAMES {
        let f = |v: f32| v.to_bits();
        let em: [u32; 32] = [
            f(0.0), f(1.0), f(0.0), f(0.5), // origin, spawn radius
            f(0.0), f(1.0), f(0.0), f(0.4), // direction, spread
            f(1.0), f(2.0), f(LIFETIME), f(LIFETIME), // speed, lifetime
            f(0.0), f(-9.8), f(0.0), f(0.1), // gravity, drag
            f(1.0), f(0.5), f(0.2), f(1.0), // colour start
            f(0.2), f(0.2), f(0.2), f(0.0), // colour end
            f(0.2), f(0.6), f(DT), f(frame as f32 * DT), // size start and end, dt, time
            EMIT, CAPACITY, frame + 1, parity,
        ];
        queue.write_buffer(&uniform, 0, &words(&em));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("sim") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("particles"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &sim_group, &[]);
            pass.set_pipeline(&emit);
            pass.dispatch_workgroups(EMIT.div_ceil(64), 1, 1);
            pass.set_pipeline(&prepare);
            pass.set_bind_group(1, &args_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&update);
            pass.dispatch_workgroups_indirect(&args, 16);
            pass.set_pipeline(&finish);
            pass.set_bind_group(1, &args_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        queue.submit(Some(encoder.finish()));

        // The same frame on the CPU.
        let e = EMIT.min(dead);
        dead -= e;
        emitted_total += e;
        batches.push((e, 0));
        let alive_after_emit: u32 = batches.iter().map(|b| b.0).sum();
        for b in &mut batches {
            b.1 += 1;
        }
        let (gone, kept): (Vec<(u32, u32)>, Vec<(u32, u32)>) =
            batches.iter().copied().partition(|b| b.1 as f32 * DT >= LIFETIME * (1.0 - 1e-5));
        let died: u32 = gone.iter().map(|b| b.0).sum();
        died_total += died;
        dead += died;
        batches = kept;
        let alive: u32 = batches.iter().map(|b| b.0).sum();

        let c = read_back(device, queue, &counters)?;
        let a = read_back(device, queue, &args)?;
        let l = read_back(device, queue, &lists)?;
        let p = read_back(device, queue, &particles)?;
        let at = format!("frame {frame}");
        if c[0] != alive || c[1] != alive || c[2] != dead {
            return Err(format!("{at}: counters alive {}/{} dead {}, want alive {alive} dead {dead}", c[0], c[1], c[2]));
        }
        if a[0..4] != [6, alive, 0, 0] {
            return Err(format!("{at}: draw args {:?}, want [6, {alive}, 0, 0]", &a[0..4]));
        }
        let groups = alive_after_emit.div_ceil(64);
        if a[4..7] != [groups, 1, 1] {
            return Err(format!("{at}: update args {:?}, want [{groups}, 1, 1]", &a[4..7]));
        }
        // Survivors went to the other half; with the dead list they hold every slot exactly once.
        let next = ((1 - parity) * CAPACITY) as usize;
        let live = &l[next..next + alive as usize];
        let free = &l[(2 * CAPACITY) as usize..(2 * CAPACITY + dead) as usize];
        let mut seen = vec![0u8; CAPACITY as usize];
        for &slot in live.iter().chain(free) {
            if slot >= CAPACITY {
                return Err(format!("{at}: slot {slot} out of range"));
            }
            seen[slot as usize] += 1;
        }
        if seen.iter().any(|&n| n != 1) {
            return Err(format!("{at}: the alive and dead lists do not hold every slot exactly once"));
        }
        for &slot in live {
            let r = &p[(slot * 16) as usize..(slot * 16 + 16) as usize];
            let fl = |i: usize| f32::from_bits(r[i]);
            let (age, life) = (fl(3), fl(7));
            if !(age > 0.0 && age < life && (life - LIFETIME).abs() < 1e-6) {
                return Err(format!("{at}: slot {slot} age {age} lifetime {life}"));
            }
            let pos = [fl(0), fl(1), fl(2)];
            if !pos.iter().all(|v| v.is_finite()) || pos[1] < 0.0 {
                return Err(format!("{at}: slot {slot} position {pos:?}"));
            }
        }
        parity = 1 - parity;
    }
    if died_total == 0 || emitted_total <= CAPACITY {
        return Err(format!("the run did not recycle slots ({emitted_total} emitted, {died_total} died)"));
    }
    Ok(format!("{FRAMES} frames, {emitted_total} emitted, {died_total} recycled; counts, args and lists match"))
}
