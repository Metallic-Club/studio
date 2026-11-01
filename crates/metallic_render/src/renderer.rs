//! The GPU renderer: buffers, pipelines and the frame stages (shadows, scene into the HDR target,
//! HDR post passes, finish, LDR post passes; see [`crate::post`]).
use crate::bloom::{BLOOM_PASS, BloomPass};
use crate::camera::{Camera, FAR, NEAR};
use crate::context::{RenderContext, RenderError};
use crate::edges::EdgePass;
use crate::exposure::{AutoExposure, ExposureGpu, ExposureState};
use crate::lighting::{LightingInputs, LightingUniform, evaluate, reference_exposure};
use crate::lut::Lut3d;
use crate::meshes::{MeshKind, Vertex};
use crate::mlaa::{MLAA_PASS, MlaaPass};
use crate::post::{HDR_FORMAT, LDR_FORMAT, PostChain, PostContext, PostSlot, hdr_format_for};
use crate::scene::{InstanceRaw, Scene};
use crate::shading::{MAX_ROWS, MaterialRow, MaterialTable, Shading};
use crate::shadows::{
    Cascade, MAX_CASCADES, SHADOW_DISTANCE, SHADOW_FORMAT, ShadowMap, cascade_splits, fit_cascade,
};
use crate::skinned::{
    GpuSkinnedModel, SkinSlot, SkinStyle, SkinnedDraw, SkinnedInstance, projected_height,
    select_lod, skinned_layout,
};
use crate::sky::Sky;
use crate::tonemap::{FinishSettings, FinishUniform};
use bytemuck::{Pod, Zeroable};
use metallic_core::World;
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use wgpu::util::DeviceExt;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const COLOR_WGSL: &str = include_str!("../shaders/color.wgsl");
const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");
const FINISH_WGSL: &str = include_str!("../shaders/finish.wgsl");
const TONECURVE_WGSL: &str = include_str!("../shaders/tonecurve.wgsl");
const SCENE_WGSL: &str = include_str!("../shaders/scene.wgsl");
const SKY_WGSL: &str = include_str!("../shaders/sky.wgsl");
const LIGHTING_WGSL: &str = include_str!("../shaders/lighting.wgsl");
const SHADOW_WGSL: &str = include_str!("../shaders/shadow.wgsl");
const SKIN_COMMON_WGSL: &str = include_str!("../shaders/skin_common.wgsl");
const SKINNED_WGSL: &str = include_str!("../shaders/skinned.wgsl");

// The WGSL files each shader module is joined from.
const SCENE_PARTS: &[&str] = &[
    COLOR_WGSL,
    TONECURVE_WGSL,
    COMMON_WGSL,
    LIGHTING_WGSL,
    SKIN_COMMON_WGSL,
    SCENE_WGSL,
    SKINNED_WGSL,
];
const SKY_PARTS: &[&str] = &[
    COLOR_WGSL,
    TONECURVE_WGSL,
    COMMON_WGSL,
    LIGHTING_WGSL,
    SKY_WGSL,
];
const SHADOW_PARTS: &[&str] = &[SKIN_COMMON_WGSL, SHADOW_WGSL];
const FINISH_PARTS: &[&str] = &[COLOR_WGSL, TONECURVE_WGSL, FINISH_WGSL];
const PREPASS_WGSL: &str = include_str!("../shaders/prepass.wgsl");
const PREPASS_PARTS: &[&str] = &[
    COLOR_WGSL,
    TONECURVE_WGSL,
    COMMON_WGSL,
    SKIN_COMMON_WGSL,
    PREPASS_WGSL,
];
/// Format of the geometry prepass target (normal and linear depth).
const GEOMETRY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Overall quality preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    /// One 1024 px shadow cascade, no multisampling.
    Low,
    /// Three 2048 px shadow cascades and 4x MSAA.
    #[default]
    High,
}

/// Settings that can change at run time with [`Renderer::set_settings`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderSettings {
    /// Quality preset.
    pub quality: Quality,
}

impl RenderSettings {
    fn cascades(self) -> usize {
        match self.quality {
            Quality::Low => 1,
            Quality::High => 3,
        }
    }
    fn shadow_resolution(self) -> u32 {
        match self.quality {
            Quality::Low => 1024,
            Quality::High => 2048,
        }
    }
    fn samples(self) -> u32 {
        match self.quality {
            Quality::Low => 1,
            Quality::High => 4,
        }
    }
}

/// Byte offset of the auto exposure multiplier (`light_toon.y`) in [`Globals`].
const GLOBALS_AUTO_OFFSET: u64 = (std::mem::offset_of!(Globals, lighting)
    + std::mem::offset_of!(LightingUniform, toon)
    + 4) as u64;

/// Uniform block shared by all shaders; mirrors `Globals` in `common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    cascade_vp: [[[f32; 4]; 4]; MAX_CASCADES],
    cascade_splits: [f32; 4],
    cascade_texel: [f32; 4],
    camera_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    sky_ground: [f32; 4],
    params: [f32; 4],
    viewport: [f32; 4],
    lighting: LightingUniform,
}

const MESH_ATTRS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
const INSTANCE_ATTRS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Uint32x4
];

fn vertex_layouts() -> [wgpu::VertexBufferLayout<'static>; 2] {
    [
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &MESH_ATTRS,
        },
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<InstanceRaw>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRS,
        },
    ]
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

/// Device features GPU frame timing needs ([`Renderer::enable_gpu_timing`]).
pub(crate) const GPU_TIMING_FEATURES: wgpu::Features =
    wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

/// Two timestamps around a frame's commands, resolved into a buffer the caller maps on demand.
struct GpuTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    period_ns: f32,
    recorded: bool,
}

/// A growable GPU buffer of instances.
struct InstanceBuffer {
    buffer: wgpu::Buffer,
    capacity: usize,
    count: usize,
}

impl InstanceBuffer {
    fn new(device: &wgpu::Device, capacity: usize) -> InstanceBuffer {
        let capacity = capacity.max(16);
        InstanceBuffer {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("metallic instances"),
                size: (capacity * std::mem::size_of::<InstanceRaw>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            capacity,
            count: 0,
        }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[InstanceRaw]) {
        if data.len() > self.capacity {
            *self = InstanceBuffer::new(device, data.len().next_power_of_two());
        }
        if !data.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(data));
        }
        self.count = data.len();
    }
}

struct Pipelines {
    sky: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    blended: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    skinned: wgpu::RenderPipeline,
    skinned_toon: wgpu::RenderPipeline,
    skinned_outline: wgpu::RenderPipeline,
    skinned_shadow: wgpu::RenderPipeline,
}

/// Size-dependent targets. `hdr[0]` receives the scene; `hdr[1]` and `ldr` are made on demand
/// when the post chain has HDR or LDR passes. `finish_groups[i]` reads `hdr[i]`.
struct Attachments {
    size: (u32, u32),
    msaa_color: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
    /// Whether `depth` can be bound by post passes. Only while a pass reads depth: a sampleable
    /// multisampled depth texture renders nothing on some GL drivers.
    depth_sampleable: bool,
    hdr: [Option<wgpu::TextureView>; 2],
    ldr: [Option<wgpu::TextureView>; 2],
    finish_groups: [Option<wgpu::BindGroup>; 2],
    /// Geometry prepass target and its own single-sampled depth, made while a pass reads them.
    geometry: Option<(wgpu::TextureView, wgpu::TextureView)>,
}

/// Pipelines of the geometry prepass (`shaders/prepass.wgsl`), made on first use.
struct PrepassPipelines {
    parts: wgpu::RenderPipeline,
    skinned: wgpu::RenderPipeline,
    skinned_hull: wgpu::RenderPipeline,
}

/// The finish pass's GPU objects; pipelines are made per target format on first use.
struct FinishGpu {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    uniform: wgpu::Buffer,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    /// The grade LUT (identity until `Renderer::set_lut`), its edge length and strength.
    lut: wgpu::TextureView,
    lut_size: u32,
    lut_strength: f32,
    lut_sampler: wgpu::Sampler,
}

impl FinishGpu {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> FinishGpu {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic finish layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("metallic finish pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        FinishGpu {
            layout,
            pipeline_layout,
            module: shader(device, "metallic finish", FINISH_PARTS),
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("metallic finish uniform"),
                size: std::mem::size_of::<FinishUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            pipelines: Vec::new(),
            lut: lut_texture(device, queue, &Lut3d::identity(2)),
            lut_size: 2,
            lut_strength: 0.0,
            lut_sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("metallic lut sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }

    /// The finish bind group reading HDR target `hdr`.
    fn group(&self, device: &wgpu::Device, hdr: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("metallic finish group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.lut),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.lut_sampler),
                },
            ],
        })
    }

    fn pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let i = match self.pipelines.iter().position(|(f, _)| *f == format) {
            Some(i) => i,
            None => {
                let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("metallic finish"),
                    layout: Some(&self.pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &self.module,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &self.module,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                self.pipelines.push((format, p));
                self.pipelines.len() - 1
            }
        };
        &self.pipelines[i].1
    }
}

/// A filterable 3D texture holding `lut`.
fn lut_texture(device: &wgpu::Device, queue: &wgpu::Queue, lut: &Lut3d) -> wgpu::TextureView {
    let n = lut.size();
    let size = wgpu::Extent3d {
        width: n,
        height: n,
        depth_or_array_layers: n,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("metallic grade lut"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(&lut.texels()),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(n * 8),
            rows_per_image: Some(n),
        },
        size,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn color_target(
    device: &wgpu::Device,
    label: &str,
    size: (u32, u32),
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

struct CascadeSlot {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Draws the Parts of a world. Create with [`Renderer::new`], call [`Renderer::sync`] when the
/// world changed and [`Renderer::render`] once per frame.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    settings: RenderSettings,
    /// Environment colours and sun; change fields at any time.
    pub sky: Sky,
    /// Tone curve, grade and dither of the finish pass; change fields at any time.
    pub finish: FinishSettings,
    post: PostChain,
    finish_gpu: FinishGpu,
    /// Histogram metering; [`Renderer::set_lighting`] sets it from the `Lighting` properties.
    /// Change fields at any time.
    pub auto_exposure: AutoExposure,
    /// The fixed exposure already applied to the lights (the lighting's), metered against.
    pre_exposure: f32,
    /// Whether the device runs compute shaders (not WebGL2); without them exposure stays fixed.
    meter_supported: bool,
    /// Created on the first metered frame.
    meter: Option<ExposureGpu>,
    /// The next metered frame jumps to its target.
    meter_reset: bool,
    /// Time step for the next metered frame ([`Renderer::set_frame_time`]); measured otherwise.
    meter_dt: Option<f32>,
    meter_last: Option<std::time::Instant>,
    /// The lighting last applied by [`Renderer::set_lighting`].
    lighting: Option<LightingInputs>,
    light_uniform: LightingUniform,
    /// What the adapter supports for [`HDR_FORMAT`] (`None` for a host-provided device).
    hdr_features: Option<wgpu::TextureFormatFeatures>,
    /// The HDR target format in use (see [`hdr_format_for`]).
    hdr_format: wgpu::TextureFormat,
    /// Bumped whenever the attachments are rebuilt ([`PostContext::targets`]).
    targets: u64,
    scene: Scene,
    uploaded_version: Option<u64>,
    meshes: Vec<GpuMesh>,
    opaque: Vec<InstanceBuffer>,
    blended: InstanceBuffer,
    blended_kinds: Vec<MeshKind>,
    scratch_instances: Vec<InstanceRaw>,
    gpu_timer: Option<GpuTimer>,
    /// The GPU material table (`materials` in scene.wgsl) and its CPU copy.
    material_buffer: wgpu::Buffer,
    material_table: MaterialTable,
    scratch_blended: Vec<(f32, MeshKind, InstanceRaw)>,
    scratch_cascades: Vec<Cascade>,
    globals: wgpu::Buffer,
    global_layout: wgpu::BindGroupLayout,
    prepass: Option<PrepassPipelines>,
    shadow_layout: wgpu::BindGroupLayout,
    skin_layout: wgpu::BindGroupLayout,
    global_group: wgpu::BindGroup,
    shadow_map: ShadowMap,
    cascade_slots: Vec<CascadeSlot>,
    pipelines: Pipelines,
    attachments: Option<Attachments>,
    /// Skinned models uploaded so far, kept for the renderer's lifetime.
    skinned_models: Vec<GpuSkinnedModel>,
    /// Per-instance joint buffers, reused from frame to frame.
    skin_slots: Vec<SkinSlot>,
    /// The skinned instances given to the last [`Renderer::set_skinned`].
    skinned_draws: Vec<SkinnedDraw>,
    skin_scratch: Vec<u8>,
    /// Draw calls issued by the last [`Renderer::render`] (developer overlay).
    draws: AtomicU32,
}

impl Renderer {
    /// Creates a renderer at [`Quality::High`] that draws into targets of `surface_format`.
    pub fn new(ctx: &RenderContext, surface_format: wgpu::TextureFormat) -> Renderer {
        Renderer::with_settings(ctx, surface_format, RenderSettings::default())
    }

    /// Creates a renderer with explicit settings.
    pub fn with_settings(
        ctx: &RenderContext,
        surface_format: wgpu::TextureFormat,
        settings: RenderSettings,
    ) -> Renderer {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        let meshes = MeshKind::ALL
            .iter()
            .map(|k| {
                let data = k.build();
                GpuMesh {
                    vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("metallic mesh vertices"),
                        contents: bytemuck::cast_slice(&data.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                    indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("metallic mesh indices"),
                        contents: bytemuck::cast_slice(&data.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                    index_count: data.indices.len() as u32,
                }
            })
            .collect();
        let opaque = MeshKind::ALL
            .iter()
            .map(|_| InstanceBuffer::new(&device, 1024))
            .collect();
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let global_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic global layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic shadow layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let skin_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic skin layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let shadow_map = ShadowMap::new(&device, settings.shadow_resolution(), settings.cascades());
        let material_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic material table"),
            size: std::mem::size_of::<[MaterialRow; MAX_ROWS]>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let global_group = make_global_group(
            &device,
            &global_layout,
            &globals,
            &shadow_map,
            &material_buffer,
        );
        let cascade_slots = make_cascade_slots(&device, &shadow_layout, settings.cascades());
        let hdr_features = ctx
            .adapter
            .as_ref()
            .map(|a| a.get_texture_format_features(HDR_FORMAT));
        let hdr_format = hdr_format_for(hdr_features, settings.samples());
        if hdr_format != HDR_FORMAT {
            log::warn!("{HDR_FORMAT:?} is not renderable here; HDR falls back to {hdr_format:?}");
        }
        let pipelines = make_pipelines(
            &device,
            settings.samples(),
            hdr_format,
            &global_layout,
            &shadow_layout,
            &skin_layout,
        );
        let blended = InstanceBuffer::new(&device, 256);
        let finish_gpu = FinishGpu::new(&device, &queue);
        let mut renderer = Renderer {
            device,
            queue,
            format: surface_format,
            settings,
            sky: Sky::default(),
            finish: FinishSettings::default(),
            post: PostChain::default(),
            auto_exposure: AutoExposure {
                enabled: false,
                ..AutoExposure::default()
            },
            pre_exposure: 1.0,
            meter_supported: false,
            meter: None,
            meter_reset: true,
            meter_dt: None,
            meter_last: None,
            lighting: None,
            light_uniform: LightingUniform::zeroed(),
            finish_gpu,
            hdr_features,
            hdr_format,
            targets: 0,
            scene: Scene::new(),
            uploaded_version: None,
            meshes,
            opaque,
            blended,
            blended_kinds: Vec::new(),
            scratch_instances: Vec::new(),
            gpu_timer: None,
            material_buffer,
            material_table: MaterialTable::default(),
            scratch_blended: Vec::new(),
            scratch_cascades: Vec::new(),
            globals,
            global_layout,
            shadow_layout,
            skin_layout,
            global_group,
            shadow_map,
            cascade_slots,
            pipelines,
            attachments: None,
            prepass: None,
            skinned_models: Vec::new(),
            skin_slots: Vec::new(),
            skinned_draws: Vec::new(),
            skin_scratch: Vec::new(),
            draws: AtomicU32::new(0),
        };
        renderer.meter_supported = crate::exposure::supported(&renderer.device);
        renderer.post.push(Box::new(EdgePass::default()));
        renderer.post.push(Box::new(BloomPass::default()));
        let mut aa = MlaaPass::default();
        aa.enabled = settings.samples() == 1;
        renderer.post.push(Box::new(aa));
        renderer.set_lighting(&LightingInputs::default());
        renderer
    }

    /// The settings in use.
    pub fn settings(&self) -> RenderSettings {
        self.settings
    }

    /// Switches quality; rebuilds the shadow map, pipelines and render targets.
    pub fn set_settings(&mut self, settings: RenderSettings) {
        if settings == self.settings {
            return;
        }
        self.settings = settings;
        // Morphological AA stands in for MSAA when MSAA is off.
        if let Some(aa) = self.post.get_mut::<MlaaPass>(MLAA_PASS) {
            aa.enabled = settings.samples() == 1;
        }
        self.shadow_map = ShadowMap::new(
            &self.device,
            settings.shadow_resolution(),
            settings.cascades(),
        );
        self.global_group = make_global_group(
            &self.device,
            &self.global_layout,
            &self.globals,
            &self.shadow_map,
            &self.material_buffer,
        );
        self.cascade_slots =
            make_cascade_slots(&self.device, &self.shadow_layout, settings.cascades());
        self.hdr_format = hdr_format_for(self.hdr_features, settings.samples());
        self.pipelines = make_pipelines(
            &self.device,
            settings.samples(),
            self.hdr_format,
            &self.global_layout,
            &self.shadow_layout,
            &self.skin_layout,
        );
        self.attachments = None;
    }

    /// Whether frames are metered: [`Renderer::auto_exposure`] is on and the device runs
    /// compute shaders.
    pub fn auto_exposure_active(&self) -> bool {
        self.auto_exposure.enabled && self.meter_supported
    }

    /// Forgets the adapted exposure (after a camera cut or teleport): the next frame meters
    /// straight to its target instead of adapting.
    pub fn reset_exposure(&mut self) {
        self.meter_reset = true;
    }

    /// Time step in seconds for the next frame's exposure adaptation. Without it the renderer
    /// measures the time since the last metered frame (at most 0.25 s).
    pub fn set_frame_time(&mut self, dt: f32) {
        self.meter_dt = Some(dt);
    }

    /// What the meter last wrote (blocking GPU readback; tests and tools). `None` before the
    /// first metered frame.
    pub fn read_exposure(&self) -> Option<ExposureState> {
        self.meter.as_ref()?.read(&self.device, &self.queue)
    }

    /// Sets the grade LUT ([`crate::lut`]) and how strongly it applies, 0 (off) .. 1. `None`
    /// keeps the current table and only changes the strength. The table is uploaded here, once.
    pub fn set_lut(&mut self, lut: Option<&Lut3d>, strength: f32) {
        self.finish_gpu.lut_strength = if strength.is_finite() {
            strength.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let Some(lut) = lut else {
            return;
        };
        self.finish_gpu.lut = lut_texture(&self.device, &self.queue, lut);
        self.finish_gpu.lut_size = lut.size();
        if let Some(att) = self.attachments.as_mut() {
            for (group, hdr) in att.finish_groups.iter_mut().zip(&att.hdr) {
                if let Some(view) = hdr {
                    *group = Some(self.finish_gpu.group(&self.device, view));
                }
            }
        }
    }

    /// The grade LUT's strength (0 when off).
    pub fn lut_strength(&self) -> f32 {
        self.finish_gpu.lut_strength
    }

    /// The post chain, to add, remove or reorder full-screen passes (see [`crate::post`]).
    pub fn post_mut(&mut self) -> &mut PostChain {
        &mut self.post
    }

    /// The post chain.
    pub fn post(&self) -> &PostChain {
        &self.post
    }

    /// Format of the HDR scene target: [`HDR_FORMAT`], or [`LDR_FORMAT`] where the adapter
    /// cannot render to it (see [`crate::post::hdr_format_for`]).
    pub fn hdr_format(&self) -> wgpu::TextureFormat {
        self.hdr_format
    }

    /// Uses the 8-bit fallback scene target even where float targets work, so tests can check
    /// the fallback path on any machine.
    #[doc(hidden)]
    pub fn force_ldr_scene_target(&mut self) {
        self.hdr_features = Some(wgpu::TextureFormatFeatures {
            allowed_usages: wgpu::TextureUsages::empty(),
            flags: wgpu::TextureFormatFeatureFlags::empty(),
        });
        self.hdr_format = hdr_format_for(self.hdr_features, self.settings.samples());
        self.pipelines = make_pipelines(
            &self.device,
            self.settings.samples(),
            self.hdr_format,
            &self.global_layout,
            &self.shadow_layout,
            &self.skin_layout,
        );
        self.attachments = None;
    }

    /// Number of parts currently known to the renderer (drawn or not).
    pub fn part_count(&self) -> usize {
        self.scene.instance_count()
    }

    /// Reads all BasePart transforms, sizes, colours, materials and transparency from `world`,
    /// and its lighting (the `Lighting` service and its effect children, see
    /// [`Renderer::set_lighting`]).
    ///
    /// Only parts whose data changed are re-uploaded; an unchanged world costs one read per
    /// property and no GPU work.
    pub fn sync(&mut self, world: &World) {
        self.scene.sync(world);
        self.set_lighting(&LightingInputs::read(world));
    }

    /// Applies lighting properties: sets [`Renderer::sky`], [`Renderer::finish`], the bloom
    /// pass and the shader's lighting block. Does nothing when `inputs` equal the last ones, so
    /// direct edits of `sky` or `finish` stay until the lighting changes.
    pub fn set_lighting(&mut self, inputs: &LightingInputs) {
        if self.lighting.as_ref() == Some(inputs) {
            return;
        }
        let frame = evaluate(inputs);
        self.sky = frame.sky;
        self.finish = frame.finish;
        self.light_uniform = frame.uniform;
        self.auto_exposure = frame.auto_exposure;
        self.pre_exposure = frame.exposure;
        if let Some(bloom) = self.post.get_mut::<BloomPass>(BLOOM_PASS) {
            match frame.bloom {
                Some(b) => {
                    bloom.intensity = b.intensity;
                    bloom.size = b.size;
                    bloom.threshold = b.threshold;
                }
                None => bloom.intensity = 0.0,
            }
        }
        self.lighting = Some(inputs.clone());
    }

    /// Sets how the place Look shades (style slot 0 of the material table): its default model,
    /// material overrides and the Character row. Uploaded with the next frame, only if it
    /// changed.
    pub fn set_shading(&mut self, shading: &Shading) {
        let _ = self.material_table.set_style(0, shading);
    }

    /// Sets the shading of a per-part style slot (1 to 7; #223 assigns slots to styles). A slot
    /// of 8 or more is ignored with a warning and returns false.
    pub fn set_style_shading(&mut self, slot: usize, shading: &Shading) -> bool {
        self.material_table.set_style(slot, shading)
    }

    /// The material table the renderer draws with.
    pub fn material_table(&self) -> &MaterialTable {
        &self.material_table
    }

    /// Sets the skinned instances (characters) to draw from the next [`Renderer::render`] on.
    /// They stay until the next call; pass an empty slice to draw none. Joint matrices are
    /// copied to the GPU here. A model is uploaded the first time it appears and then kept.
    pub fn set_skinned(&mut self, instances: &[SkinnedInstance<'_>]) {
        self.skinned_draws.clear();
        for (slot, inst) in instances.iter().enumerate() {
            let model = match self
                .skinned_models
                .iter()
                .position(|m| Arc::ptr_eq(&m.model, inst.model))
            {
                Some(i) => i,
                None => {
                    self.skinned_models
                        .push(GpuSkinnedModel::upload(&self.device, inst.model));
                    self.skinned_models.len() - 1
                }
            };
            SkinSlot::write(
                &mut self.skin_slots,
                slot,
                &self.device,
                &self.queue,
                &self.skin_layout,
                inst,
                &mut self.skin_scratch,
            );
            self.skinned_draws.push(SkinnedDraw {
                model,
                slot,
                center: inst.center,
                height: inst.height,
                style: inst.style,
                lod: 0,
            });
        }
    }

    /// Picks each skinned instance's level of detail for this view.
    fn select_skinned_lods(&mut self, camera: &Camera, viewport_height: u32) {
        for d in &mut self.skinned_draws {
            let px = projected_height(camera, viewport_height, d.center, d.height);
            d.lod = select_lod(px, self.skinned_models[d.model].lods.len());
        }
    }

    /// The level of detail each skinned instance was drawn with in the last frame (for tests
    /// and diagnostics).
    pub fn skinned_lods(&self) -> Vec<usize> {
        self.skinned_draws.iter().map(|d| d.lod).collect()
    }

    fn upload_scene(&mut self) {
        if self.uploaded_version == Some(self.scene.version()) {
            return;
        }
        let mut data = std::mem::take(&mut self.scratch_instances);
        for kind in MeshKind::ALL {
            data.clear();
            data.extend(
                self.scene
                    .opaque(kind)
                    .filter(|p| !p.is_invisible())
                    .map(|p| p.instance),
            );
            self.opaque[kind.index()].upload(&self.device, &self.queue, &data);
        }
        self.scratch_instances = data;
        self.uploaded_version = Some(self.scene.version());
    }

    fn ensure_attachments(&mut self, size: (u32, u32)) {
        let samples = self.settings.samples();
        let sampleable = self.post.reads_depth();
        if let Some(att) = self.attachments.as_mut()
            && att.size == size
        {
            if att.depth_sampleable != sampleable {
                att.depth = make_depth(&self.device, size, samples, sampleable);
                att.depth_sampleable = sampleable;
            }
            return;
        }
        let extent = wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        };
        let depth = make_depth(&self.device, size, samples, sampleable);
        let msaa_color = (samples > 1).then(|| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("metallic msaa color"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: self.hdr_format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        });
        self.targets += 1;
        self.attachments = Some(Attachments {
            size,
            msaa_color,
            depth,
            depth_sampleable: sampleable,
            hdr: [None, None],
            ldr: [None, None],
            finish_groups: [None, None],
            geometry: None,
        });
        self.ensure_hdr(0);
    }

    /// Makes HDR target `i` and the finish bind group that reads it.
    fn ensure_hdr(&mut self, i: usize) {
        let Some(att) = self.attachments.as_mut() else {
            return;
        };
        if att.hdr[i].is_some() {
            return;
        }
        let view = color_target(&self.device, "metallic hdr", att.size, self.hdr_format);
        att.finish_groups[i] = Some(self.finish_gpu.group(&self.device, &view));
        att.hdr[i] = Some(view);
    }

    /// Makes the post targets the current chain needs.
    fn ensure_post_targets(&mut self) {
        if self.post.active(PostSlot::Hdr) > 0 {
            self.ensure_hdr(1);
        }
        let ldr = self.post.active(PostSlot::Ldr);
        let Some(att) = self.attachments.as_mut() else {
            return;
        };
        // n LDR passes need the finish output plus one spare when n >= 2.
        for i in 0..ldr.min(2) {
            if att.ldr[i].is_none() {
                att.ldr[i] = Some(color_target(
                    &self.device,
                    "metallic ldr",
                    att.size,
                    LDR_FORMAT,
                ));
            }
        }
    }

    fn write_globals(&self, camera: &Camera, size: (u32, u32), cascades: &[Cascade]) -> Globals {
        let aspect = size.0 as f32 / size.1 as f32;
        let vp = camera.view_proj(aspect);
        let count = self.settings.cascades();
        let splits = cascade_splits(NEAR, SHADOW_DISTANCE.min(FAR), count);
        let mut cascade_vp = [[[0.0; 4]; 4]; MAX_CASCADES];
        let mut split_arr = [f32::MAX; 4];
        let mut texel = [1.0; 4];
        for (i, c) in cascades.iter().enumerate().take(count) {
            cascade_vp[i] = c.view_proj.to_cols_array_2d();
            split_arr[i] = splits[i];
            texel[i] = c.texel_world;
        }
        let p = |c: [f32; 3], w: f32| [c[0], c[1], c[2], w];
        Globals {
            view_proj: vp.to_cols_array_2d(),
            inv_view_proj: vp.inverse().to_cols_array_2d(),
            cascade_vp,
            cascade_splits: split_arr,
            cascade_texel: texel,
            camera_pos: [
                camera.position.x,
                camera.position.y,
                camera.position.z,
                self.sky.fog_density,
            ],
            sun_dir: p(self.sky.to_sun.to_array(), 0.0),
            sun_color: p(self.sky.sun_color, 0.0),
            sky_zenith: p(self.sky.zenith, 0.0),
            sky_horizon: p(self.sky.horizon, 0.0),
            sky_ground: p(self.sky.ground, 0.0),
            // y, z: what `unfinish` needs to invert the finish pass's curve and exposure.
            params: [
                count as f32,
                self.finish.curve.index() as f32,
                self.sky.exposure * self.finish.total_exposure(),
                self.shadow_map.resolution as f32,
            ],
            viewport: [
                size.0 as f32,
                size.1 as f32,
                1.0 / size.0.max(1) as f32,
                1.0 / size.1.max(1) as f32,
            ],
            lighting: self.light_uniform,
        }
    }

    /// Draws the synced scene into `target` (a single-sample view of the surface format given to
    /// [`Renderer::new`]) as seen by `camera`. `size` is the target size in pixels; a zero size
    /// (a minimised window) draws nothing.
    pub fn render(&mut self, target: &wgpu::TextureView, camera: &Camera, size: (u32, u32)) {
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        self.draws.store(0, Ordering::Relaxed);
        self.ensure_attachments(size);
        self.upload_scene();
        if self.material_table.take_dirty() {
            self.queue.write_buffer(
                &self.material_buffer,
                0,
                bytemuck::cast_slice(self.material_table.rows().as_slice()),
            );
        }
        self.select_skinned_lods(camera, size.1);

        // Blended parts are re-sorted every frame because the order depends on the camera.
        // The scratch vectors keep their capacity from frame to frame.
        let mut sorted = std::mem::take(&mut self.scratch_blended);
        sorted.clear();
        let eye = camera.position;
        sorted.extend(
            self.scene
                .transparent()
                .map(|p| (p.centre().distance_squared(eye), p.kind, p.instance)),
        );
        sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut instances = std::mem::take(&mut self.scratch_instances);
        instances.clear();
        instances.extend(sorted.iter().map(|s| s.2));
        self.blended_kinds.clear();
        self.blended_kinds.extend(sorted.iter().map(|s| s.1));
        self.blended.upload(&self.device, &self.queue, &instances);
        self.scratch_instances = instances;
        self.scratch_blended = sorted;

        // One fit per cascade per frame; shared by the uniform block and the shadow passes.
        let aspect = size.0 as f32 / size.1 as f32;
        let count = self.settings.cascades();
        let splits = cascade_splits(NEAR, SHADOW_DISTANCE.min(FAR), count);
        let mut cascades = std::mem::take(&mut self.scratch_cascades);
        cascades.clear();
        cascades.extend((0..count).map(|i| {
            let near = if i == 0 { NEAR } else { splits[i - 1] };
            fit_cascade(
                camera,
                aspect,
                near,
                splits[i],
                self.sky.to_sun,
                self.shadow_map.resolution,
            )
        }));
        let globals = self.write_globals(camera, size, &cascades);
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        for (slot, c) in self.cascade_slots.iter().zip(&cascades) {
            self.queue.write_buffer(
                &slot.buffer,
                0,
                bytemuck::bytes_of(&c.view_proj.to_cols_array_2d()),
            );
        }
        self.scratch_cascades = cascades;
        self.ensure_post_targets();
        let geometry = self.ensure_geometry();

        let Some(att) = self.attachments.as_ref() else {
            return;
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("metallic frame"),
            });
        if let Some(t) = &self.gpu_timer {
            encoder.write_timestamp(&t.set, 0);
        }
        self.apply_exposure(&mut encoder);

        for (i, slot) in self.cascade_slots.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("metallic shadow pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_map.layer_views[i],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipelines.shadow);
            pass.set_bind_group(0, &slot.bind_group, &[]);
            self.draw_opaque(&mut pass);
            pass.set_pipeline(&self.pipelines.skinned_shadow);
            self.draw_skinned(&mut pass, None);
        }
        if geometry {
            self.draw_prepass(&mut encoder, att);
        }

        let keep_depth = self.post.reads_depth();
        {
            // Made with the attachments; nothing to draw into otherwise.
            let Some(hdr) = att.hdr[0].as_ref() else {
                return;
            };
            let (view, resolve) = match &att.msaa_color {
                Some(msaa) => (msaa, Some(hdr)),
                None => (hdr, None),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("metallic main pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: if resolve.is_some() {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &att.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: if keep_depth {
                            wgpu::StoreOp::Store
                        } else {
                            wgpu::StoreOp::Discard
                        },
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.global_group, &[]);
            pass.set_pipeline(&self.pipelines.sky);
            pass.draw(0..3, 0..1);
            self.count_draw();
            pass.set_pipeline(&self.pipelines.opaque);
            self.draw_opaque(&mut pass);
            pass.set_pipeline(&self.pipelines.skinned);
            self.draw_skinned(&mut pass, Some(SkinStyle::Standard));
            pass.set_pipeline(&self.pipelines.skinned_toon);
            self.draw_skinned(&mut pass, Some(SkinStyle::Toon));
            pass.set_pipeline(&self.pipelines.skinned_outline);
            self.draw_skinned(&mut pass, Some(SkinStyle::Toon));
            self.draw_blended(&mut pass);
        }
        // Meter the scene before the HDR passes: they may rewrite alpha, which marks toon
        // pixels the meter must skip.
        self.meter_frame(&mut encoder);
        self.post_and_finish(&mut encoder, target, camera, keep_depth, geometry);
        if let Some(t) = &mut self.gpu_timer {
            encoder.write_timestamp(&t.set, 1);
            encoder.resolve_query_set(&t.set, 0..2, &t.resolve, 0);
            encoder.copy_buffer_to_buffer(&t.resolve, 0, &t.readback, 0, 16);
            t.recorded = true;
        }
        self.queue.submit(Some(encoder.finish()));
    }

    /// Turns on GPU frame timing: every [`Renderer::render`] then writes a timestamp before its
    /// first and after its last command. Returns false when the device lacks timestamp queries
    /// (it needs `TIMESTAMP_QUERY` and `TIMESTAMP_QUERY_INSIDE_ENCODERS`; [`RenderContext`]
    /// asks for both where the adapter has them). For tools and benchmarks.
    pub fn enable_gpu_timing(&mut self) -> bool {
        if !self.device.features().contains(GPU_TIMING_FEATURES) {
            return false;
        }
        if self.gpu_timer.is_none() {
            let buffer = |label, usage| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: 16,
                    usage,
                    mapped_at_creation: false,
                })
            };
            self.gpu_timer = Some(GpuTimer {
                set: self.device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("metallic frame timer"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                }),
                resolve: buffer(
                    "metallic frame timer resolve",
                    wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                ),
                readback: buffer(
                    "metallic frame timer readback",
                    wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                ),
                period_ns: self.queue.get_timestamp_period(),
                recorded: false,
            });
        }
        true
    }

    /// GPU time of the last rendered frame in milliseconds, from its two timestamps. Waits for
    /// the GPU; `None` when timing is off or no frame was rendered since it was turned on.
    pub fn read_gpu_time_ms(&mut self) -> Option<f64> {
        let t = self.gpu_timer.as_mut()?;
        if !t.recorded {
            return None;
        }
        let slice = t.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        let ticks: Option<[u64; 2]> = slice
            .get_mapped_range()
            .ok()
            .map(|data| bytemuck::pod_read_unaligned(&data[..16]));
        t.readback.unmap();
        let ticks = ticks?;
        let ns = ticks[1].wrapping_sub(ticks[0]) as f64 * f64::from(t.period_ns);
        Some(ns / 1e6)
    }

    /// Copies the meter's multiplier into this frame's uniforms (recorded before any pass).
    /// Skipped while a reset is pending: that frame uses the fixed exposure, not a stale one.
    fn apply_exposure(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.auto_exposure_active() || self.meter_reset {
            return;
        }
        if let Some(meter) = &self.meter {
            encoder.copy_buffer_to_buffer(&meter.state, 0, &self.globals, GLOBALS_AUTO_OFFSET, 4);
            encoder.copy_buffer_to_buffer(
                &meter.state,
                0,
                &self.finish_gpu.uniform,
                FinishUniform::AUTO_OFFSET,
                4,
            );
        }
    }

    /// Meters the scene image (HDR target 0, before the HDR passes) for the next frame. Toon
    /// pixels (alpha 0) are skipped: their light is chosen through the exposure, so metering
    /// them would feed the meter its own output.
    fn meter_frame(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.auto_exposure_active() {
            // Turning metering back on starts from the target, not a stale state.
            self.meter_last = None;
            self.meter_reset = true;
            return;
        }
        let Some((hdr, size)) = self
            .attachments
            .as_ref()
            .and_then(|a| Some((a.hdr[0].clone()?, a.size)))
        else {
            return;
        };
        let now = std::time::Instant::now();
        let dt = self.meter_dt.take().unwrap_or_else(|| {
            self.meter_last
                .map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(0.25))
        });
        self.meter_last = Some(now);
        let reset = std::mem::take(&mut self.meter_reset);
        let meter = self
            .meter
            .get_or_insert_with(|| ExposureGpu::new(&self.device));
        meter.record(
            &self.device,
            &self.queue,
            encoder,
            &hdr,
            self.targets,
            size,
            &self.auto_exposure,
            self.pre_exposure,
            self.sky.exposure * self.finish.exposure,
            reference_exposure(),
            dt,
            reset,
        );
    }

    /// Runs the HDR passes, the finish pass and the LDR passes; the last of them writes
    /// `target`.
    fn post_and_finish(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        camera: &Camera,
        keep_depth: bool,
        geometry: bool,
    ) {
        let Some(att) = self.attachments.as_ref() else {
            return;
        };
        let size = att.size;
        let samples = self.settings.samples();
        let mut chain = std::mem::take(&mut self.post);

        // HDR passes ping-pong between hdr[0] and hdr[1].
        let mut cur = 0;
        for pass in chain.active_mut(PostSlot::Hdr) {
            let (Some(input), Some(output)) = (&att.hdr[cur], &att.hdr[1 - cur]) else {
                break;
            };
            pass.record(&mut PostContext {
                device: &self.device,
                queue: &self.queue,
                encoder: &mut *encoder,
                input,
                output,
                output_format: self.hdr_format,
                hdr_format: self.hdr_format,
                targets: self.targets,
                size,
                depth: keep_depth.then_some(&att.depth),
                depth_samples: samples,
                geometry: att.geometry.as_ref().filter(|_| geometry).map(|g| &g.0),
                camera,
            });
            cur = 1 - cur;
        }

        let ldr_count = chain.active(PostSlot::Ldr);
        let (finish_view, finish_format) = match &att.ldr[0] {
            Some(v) if ldr_count > 0 => (v, LDR_FORMAT),
            _ => (target, self.format),
        };
        let uniform = FinishUniform::new(
            &self.finish,
            self.sky.exposure,
            finish_format.is_srgb(),
            (self.finish_gpu.lut_strength, self.finish_gpu.lut_size),
        );
        self.queue
            .write_buffer(&self.finish_gpu.uniform, 0, bytemuck::bytes_of(&uniform));
        if let Some(group) = &att.finish_groups[cur] {
            let pipeline = self.finish_gpu.pipeline(&self.device, finish_format);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("metallic finish pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: finish_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
            self.count_draw();
        }

        // LDR passes: ldr[0] -> ldr[1] -> ldr[0] ..., the last one into the target.
        let mut cur = 0;
        for (i, pass) in chain.active_mut(PostSlot::Ldr).enumerate() {
            let last = i + 1 == ldr_count;
            let Some(input) = &att.ldr[cur] else {
                break;
            };
            let (output, output_format) = match &att.ldr[1 - cur] {
                Some(v) if !last => (v, LDR_FORMAT),
                _ => (target, self.format),
            };
            pass.record(&mut PostContext {
                device: &self.device,
                queue: &self.queue,
                encoder: &mut *encoder,
                input,
                output,
                output_format,
                hdr_format: self.hdr_format,
                targets: self.targets,
                size,
                depth: keep_depth.then_some(&att.depth),
                depth_samples: samples,
                geometry: att.geometry.as_ref().filter(|_| geometry).map(|g| &g.0),
                camera,
            });
            cur = 1 - cur;
        }
        self.post = chain;
    }

    fn count_draw(&self) {
        self.draws.fetch_add(1, Ordering::Relaxed);
    }

    /// Draw calls the last [`Renderer::render`] issued (sky, scene and fullscreen passes; the
    /// post chain is not counted).
    pub fn draw_calls(&self) -> u32 {
        self.draws.load(Ordering::Relaxed)
    }
    /// Makes the geometry prepass pipelines and targets when an enabled pass reads them and
    /// the adapter can render [`GEOMETRY_FORMAT`]; true when the prepass should run.
    fn ensure_geometry(&mut self) -> bool {
        if !self.post.reads_geometry() || self.hdr_format != HDR_FORMAT {
            return false;
        }
        if self.prepass.is_none() {
            self.prepass = Some(make_prepass(
                &self.device,
                &self.global_layout,
                &self.skin_layout,
            ));
        }
        let Some(att) = self.attachments.as_mut() else {
            return false;
        };
        if att.geometry.is_none() {
            att.geometry = Some((
                color_target(&self.device, "metallic geometry", att.size, GEOMETRY_FORMAT),
                make_depth(&self.device, att.size, 1, false),
            ));
        }
        true
    }

    /// The geometry prepass: opaque parts and skinned meshes into the geometry target.
    fn draw_prepass(&self, encoder: &mut wgpu::CommandEncoder, att: &Attachments) {
        let (Some(p), Some((color, depth))) = (&self.prepass, &att.geometry) else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("metallic geometry prepass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.global_group, &[]);
        pass.set_pipeline(&p.parts);
        self.draw_opaque(&mut pass);
        pass.set_pipeline(&p.skinned);
        self.draw_skinned(&mut pass, Some(SkinStyle::Standard));
        pass.set_pipeline(&p.skinned_hull);
        self.draw_skinned(&mut pass, Some(SkinStyle::Toon));
    }

    fn draw_opaque(&self, pass: &mut wgpu::RenderPass<'_>) {
        for kind in MeshKind::ALL {
            let inst = &self.opaque[kind.index()];
            if inst.count == 0 {
                continue;
            }
            let mesh = &self.meshes[kind.index()];
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_vertex_buffer(1, inst.buffer.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..mesh.index_count, 0, 0..inst.count as u32);
            self.count_draw();
        }
    }

    /// Draws the skinned instances of `style` (all of them for `None`) with the pipeline already
    /// set.
    fn draw_skinned(&self, pass: &mut wgpu::RenderPass<'_>, style: Option<SkinStyle>) {
        for d in &self.skinned_draws {
            if style.is_some_and(|s| s != d.style) {
                continue;
            }
            let Some(lod) = self.skinned_models[d.model].lods.get(d.lod) else {
                continue;
            };
            pass.set_bind_group(1, &self.skin_slots[d.slot].bind_group, &[]);
            pass.set_vertex_buffer(0, lod.vertices.slice(..));
            pass.set_index_buffer(lod.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..lod.index_count, 0, 0..1);
            self.count_draw();
        }
    }

    fn draw_blended(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.blended.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipelines.blended);
        let stride = std::mem::size_of::<InstanceRaw>() as u64;
        let mut bound: Option<MeshKind> = None;
        for (i, kind) in self.blended_kinds.iter().enumerate() {
            let mesh = &self.meshes[kind.index()];
            if bound != Some(*kind) {
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint16);
                bound = Some(*kind);
            }
            let offset = i as u64 * stride;
            pass.set_vertex_buffer(1, self.blended.buffer.slice(offset..offset + stride));
            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            self.count_draw();
        }
    }

    /// Draws one throw-away frame offscreen at `size` and waits for it, so pipeline
    /// compilation, render-target creation and instance-buffer growth happen now instead of in
    /// the first visible frame. Call after [`Renderer::sync`].
    pub fn prewarm(&mut self, size: (u32, u32)) {
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("metallic prewarm"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.render(&view, &Camera::default(), size);
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Renders one frame offscreen and returns it as tightly packed RGBA8 bytes
    /// (`size.0 * size.1 * 4`), already encoded for display. Used for screenshots and tests.
    pub fn capture(&mut self, camera: &Camera, size: (u32, u32)) -> Result<Vec<u8>, RenderError> {
        let (w, h) = size;
        if w == 0 || h == 0 {
            return Err(RenderError::InvalidSize);
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("metallic capture"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        if self.auto_exposure_active() {
            // A still: meter this view on its own state first, then render it with the
            // converged exposure, so the same inputs always give the same image. The live
            // adaptation is saved and restored around it.
            let saved = (self.meter_reset, self.meter_dt, self.meter_last);
            let meter = self
                .meter
                .get_or_insert_with(|| ExposureGpu::new(&self.device));
            meter.save(&self.device, &self.queue);
            self.meter_reset = true;
            self.render(&view, camera, size);
            self.meter_dt = Some(0.0);
            self.render(&view, camera, size);
            if let Some(meter) = &self.meter {
                meter.restore(&self.device, &self.queue);
            }
            (self.meter_reset, self.meter_dt, self.meter_last) = saved;
        } else {
            self.render(&view, camera, size);
        }

        let row_bytes = w * 4;
        let padded = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("metallic readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| RenderError::Readback(e.to_string()))?
            .map_err(|e| RenderError::Readback(e.to_string()))?;

        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        let mut out = Vec::with_capacity((row_bytes * h) as usize);
        for row in mapped.chunks_exact(padded as usize) {
            out.extend_from_slice(&row[..row_bytes as usize]);
        }
        drop(mapped);
        buffer.unmap();
        if matches!(
            self.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            for px in out.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
        }
        Ok(out)
    }
}

fn make_global_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    shadow: &ShadowMap,
    materials: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("metallic globals group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&shadow.array_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&shadow.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: materials.as_entire_binding(),
            },
        ],
    })
}

fn make_cascade_slots(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    count: usize,
) -> Vec<CascadeSlot> {
    (0..count)
        .map(|_| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("metallic cascade matrix"),
                size: 64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("metallic cascade group"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            CascadeSlot { buffer, bind_group }
        })
        .collect()
}

/// The scene depth target; `sampleable` adds `TEXTURE_BINDING` for post passes that read it.
fn make_depth(
    device: &wgpu::Device,
    size: (u32, u32),
    samples: u32,
    sampleable: bool,
) -> wgpu::TextureView {
    let usage = if sampleable {
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
    } else {
        wgpu::TextureUsages::RENDER_ATTACHMENT
    };
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("metallic depth"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn shader(device: &wgpu::Device, label: &str, parts: &[&str]) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(parts.join("\n"))),
    })
}

fn make_prepass(
    device: &wgpu::Device,
    global_layout: &wgpu::BindGroupLayout,
    skin_layout: &wgpu::BindGroupLayout,
) -> PrepassPipelines {
    let module = shader(device, "metallic geometry prepass", PREPASS_PARTS);
    let parts_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("metallic prepass layout"),
        bind_group_layouts: &[Some(global_layout)],
        immediate_size: 0,
    });
    let skinned_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("metallic prepass skinned layout"),
        bind_group_layouts: &[Some(global_layout), Some(skin_layout)],
        immediate_size: 0,
    });
    let pipeline = |label: &str,
                    layout: &wgpu::PipelineLayout,
                    vs: &str,
                    fs: &str,
                    buffers: &[Option<wgpu::VertexBufferLayout<'_>>]| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: GEOMETRY_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    PrepassPipelines {
        parts: pipeline(
            "metallic prepass parts",
            &parts_layout,
            "vs_part",
            "fs_edges",
            &vertex_layouts().map(Some),
        ),
        skinned: pipeline(
            "metallic prepass skinned",
            &skinned_pipeline_layout,
            "vs_skin",
            "fs_edges",
            &[Some(skinned_layout())],
        ),
        skinned_hull: pipeline(
            "metallic prepass skinned hull",
            &skinned_pipeline_layout,
            "vs_skin",
            "fs_hull",
            &[Some(skinned_layout())],
        ),
    }
}

fn make_pipelines(
    device: &wgpu::Device,
    samples: u32,
    format: wgpu::TextureFormat,
    global_layout: &wgpu::BindGroupLayout,
    shadow_layout: &wgpu::BindGroupLayout,
    skin_layout: &wgpu::BindGroupLayout,
) -> Pipelines {
    let scene_module = shader(device, "metallic scene", SCENE_PARTS);
    let sky_module = shader(device, "metallic sky", SKY_PARTS);
    // Every scene pipeline writes linear light into the HDR target (`format`).
    let shadow_module = shader(device, "metallic shadow", SHADOW_PARTS);

    let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("metallic main layout"),
        bind_group_layouts: &[Some(global_layout)],
        immediate_size: 0,
    });
    let shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("metallic shadow pipeline layout"),
        bind_group_layouts: &[Some(shadow_layout)],
        immediate_size: 0,
    });
    let skinned_layout_main = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("metallic skinned layout"),
        bind_group_layouts: &[Some(global_layout), Some(skin_layout)],
        immediate_size: 0,
    });
    let skinned_layout_shadow = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("metallic skinned shadow layout"),
        bind_group_layouts: &[Some(shadow_layout), Some(skin_layout)],
        immediate_size: 0,
    });
    let multisample = wgpu::MultisampleState {
        count: samples,
        mask: !0,
        alpha_to_coverage_enabled: false,
    };
    let depth = |write: bool, compare: wgpu::CompareFunction| wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    };
    let scene_pipeline = |label: &str, blend: Option<wgpu::BlendState>, write_depth: bool| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&main_layout),
            vertex: wgpu::VertexState {
                module: &scene_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &vertex_layouts().map(Some),
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(depth(write_depth, wgpu::CompareFunction::Less)),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &scene_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let sky = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("metallic sky"),
        layout: Some(&main_layout),
        vertex: wgpu::VertexState {
            module: &sky_module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(depth(false, wgpu::CompareFunction::Always)),
        multisample,
        fragment: Some(wgpu::FragmentState {
            module: &sky_module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let shadow_pipeline =
        |label: &str,
         layout: &wgpu::PipelineLayout,
         entry: &str,
         buffers: &[Option<wgpu::VertexBufferLayout<'_>>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shadow_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: Some(wgpu::Face::Front),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: SHADOW_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 2,
                        slope_scale: 2.0,
                        clamp: 0.0,
                    },
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        };
    let shadow = shadow_pipeline(
        "metallic shadow",
        &shadow_pipeline_layout,
        "vs_main",
        &vertex_layouts().map(Some),
    );
    let skinned_shadow = shadow_pipeline(
        "metallic skinned shadow",
        &skinned_layout_shadow,
        "vs_skinned",
        &[Some(skinned_layout())],
    );
    let skinned_pipeline = |label: &str, vs: &str, fs: &str, cull: wgpu::Face| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&skinned_layout_main),
            vertex: wgpu::VertexState {
                module: &scene_module,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers: &[Some(skinned_layout())],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(cull),
                ..Default::default()
            },
            depth_stencil: Some(depth(true, wgpu::CompareFunction::Less)),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &scene_module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let skinned = skinned_pipeline(
        "metallic skinned",
        "vs_skinned",
        "fs_main",
        wgpu::Face::Back,
    );
    let skinned_toon = skinned_pipeline(
        "metallic skinned toon",
        "vs_skinned",
        "fs_toon",
        wgpu::Face::Back,
    );
    // Inverted hull: only the pushed-out back faces show, around the silhouette and creases.
    let skinned_outline = skinned_pipeline(
        "metallic skinned outline",
        "vs_outline",
        "fs_outline",
        wgpu::Face::Front,
    );
    Pipelines {
        sky,
        opaque: scene_pipeline("metallic opaque", None, true),
        blended: scene_pipeline(
            "metallic blended",
            Some(wgpu::BlendState::ALPHA_BLENDING),
            false,
        ),
        shadow,
        skinned,
        skinned_toon,
        skinned_outline,
        skinned_shadow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses and validates `parts` joined, as `shader` would hand them to wgpu.
    pub(crate) fn validate_wgsl(label: &str, parts: &[&str]) {
        let source = parts.join(
            "
",
        );
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
    }

    /// Every shader module the renderer builds parses and validates (no GPU needed), so a
    /// merge that breaks a shader fails here rather than at pipeline creation.
    #[test]
    fn every_shader_module_validates() {
        validate_wgsl("scene", SCENE_PARTS);
        validate_wgsl("sky", SKY_PARTS);
        validate_wgsl("shadow", SHADOW_PARTS);
        validate_wgsl("finish", FINISH_PARTS);
        validate_wgsl("prepass", PREPASS_PARTS);
    }
}
