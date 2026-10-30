//! Builds every style-shader sample as a real wgpu pipeline on the chosen backend and reports errors.
//!
//! Usage: `style_shaders_check <dx12|vulkan>`. Exit code 0 only if every module and pipeline was created
//! without a validation error. Pipelines use automatic layouts, so this also checks that the shaders'
//! resource bindings are consistent with what wgpu derives from them.

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
    let Ok((device, _queue)) =
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

    println!("{failures} failure(s)");
    if failures == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
