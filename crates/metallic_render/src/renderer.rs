//! The GPU renderer: buffers, pipelines and the frame stages (shadows, scene into the HDR target,
//! HDR post passes, finish, LDR post passes; see [`crate::post`]).
use crate::bloom::{BLOOM_PASS, BloomPass};
use crate::camera::{Camera, FAR, NEAR};
use crate::context::{RenderContext, RenderError};
use crate::lighting::{LightingInputs, LightingUniform, evaluate};
use crate::meshes::{MeshKind, Vertex};
use crate::post::{HDR_FORMAT, LDR_FORMAT, PostChain, PostContext, PostSlot, hdr_format_for};
use crate::scene::{InstanceRaw, Scene};
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

/// Overall quality preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    /// One 1024 px shadow cascade, no multisampling.
    Low,
    /// Three 2048 px shadow cascades and 4x MSAA (lowered to the highest count the scene
    /// target and depth formats support; see [`Renderer::msaa_samples`]).
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
    /// The sample count the preset asks for; [`supported_samples`] lowers it to what the
    /// formats support.
    fn samples(self) -> u32 {
        match self.quality {
            Quality::Low => 1,
            Quality::High => 4,
        }
    }
}

/// The highest sample count up to `requested` that a colour target with `color` flags can
/// render and resolve and a depth target with `depth` flags can render; 1 always works.
fn supported_samples(
    requested: u32,
    color: wgpu::TextureFormatFeatureFlags,
    depth: wgpu::TextureFormatFeatureFlags,
) -> u32 {
    [16, 8, 4, 2]
        .into_iter()
        .filter(|&n| n <= requested)
        .find(|&n| {
            color.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
                && color.sample_count_supported(n)
                && depth.sample_count_supported(n)
        })
        .unwrap_or(1)
}

/// What the device accepts for `format`, the way wgpu validates it: the adapter's own
/// features when the device has `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` or is a downlevel
/// device, the WebGPU guarantees otherwise (and when no adapter is known).
fn device_format_features(
    device: &wgpu::Device,
    adapter: Option<&wgpu::Adapter>,
    format: wgpu::TextureFormat,
) -> wgpu::TextureFormatFeatures {
    let features = device.features();
    if let Some(a) = adapter {
        let adapter_specific =
            features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
        let downlevel = !a
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::WEBGPU_TEXTURE_FORMAT_SUPPORT);
        if adapter_specific || downlevel {
            return a.get_texture_format_features(format);
        }
    }
    format.guaranteed_format_features(features)
}

/// The scene target format ([`hdr_format_for`]) and the MSAA sample count to use with it.
fn scene_targets(
    device: &wgpu::Device,
    adapter: Option<&wgpu::Adapter>,
    hdr_features: Option<wgpu::TextureFormatFeatures>,
    settings: RenderSettings,
) -> (wgpu::TextureFormat, u32) {
    let requested = settings.samples();
    let format = hdr_format_for(hdr_features, requested);
    let samples = supported_samples(
        requested,
        device_format_features(device, adapter, format).flags,
        device_format_features(device, adapter, DEPTH_FORMAT).flags,
    );
    if samples != requested {
        log::warn!("{requested}x MSAA is not supported for {format:?}; using {samples}x");
    }
    (format, samples)
}

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
    2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4
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
}

/// The finish pass's GPU objects; pipelines are made per target format on first use.
struct FinishGpu {
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    uniform: wgpu::Buffer,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}

impl FinishGpu {
    fn new(device: &wgpu::Device) -> FinishGpu {
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
        }
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
/// world changed and [`Renderer::render`] once per frame. `render` draws with
/// [`Camera::sanitized`], so a non-finite camera position or field of view falls back to a
/// finite one.
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
    /// The lighting last applied by [`Renderer::set_lighting`].
    lighting: Option<LightingInputs>,
    light_uniform: LightingUniform,
    /// What the adapter supports for [`HDR_FORMAT`] (`None` for a host-provided device).
    hdr_features: Option<wgpu::TextureFormatFeatures>,
    /// The HDR target format in use (see [`hdr_format_for`]).
    hdr_format: wgpu::TextureFormat,
    /// The context's adapter, to check MSAA support when the settings change.
    adapter: Option<wgpu::Adapter>,
    /// MSAA sample count of the scene pass (see [`Renderer::msaa_samples`]).
    samples: u32,
    /// Bumped whenever the attachments are rebuilt ([`PostContext::targets`]).
    targets: u64,
    scene: Scene,
    uploaded_version: Option<u64>,
    meshes: Vec<GpuMesh>,
    opaque: Vec<InstanceBuffer>,
    blended: InstanceBuffer,
    blended_kinds: Vec<MeshKind>,
    scratch_instances: Vec<InstanceRaw>,
    scratch_blended: Vec<(f32, MeshKind, InstanceRaw)>,
    scratch_cascades: Vec<Cascade>,
    globals: wgpu::Buffer,
    global_layout: wgpu::BindGroupLayout,
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
        let global_group = make_global_group(&device, &global_layout, &globals, &shadow_map);
        let cascade_slots = make_cascade_slots(&device, &shadow_layout, settings.cascades());
        let hdr_features = ctx
            .adapter
            .as_ref()
            .map(|a| a.get_texture_format_features(HDR_FORMAT));
        let adapter = ctx.adapter.clone();
        let (hdr_format, samples) =
            scene_targets(&device, adapter.as_ref(), hdr_features, settings);
        if hdr_format != HDR_FORMAT {
            log::warn!("{HDR_FORMAT:?} is not renderable here; HDR falls back to {hdr_format:?}");
        }
        let pipelines = make_pipelines(
            &device,
            samples,
            hdr_format,
            &global_layout,
            &shadow_layout,
            &skin_layout,
        );
        let blended = InstanceBuffer::new(&device, 256);
        let finish_gpu = FinishGpu::new(&device);
        let mut renderer = Renderer {
            device,
            queue,
            format: surface_format,
            settings,
            sky: Sky::default(),
            finish: FinishSettings::default(),
            post: PostChain::default(),
            lighting: None,
            light_uniform: LightingUniform::zeroed(),
            finish_gpu,
            hdr_features,
            hdr_format,
            adapter,
            samples,
            targets: 0,
            scene: Scene::new(),
            uploaded_version: None,
            meshes,
            opaque,
            blended,
            blended_kinds: Vec::new(),
            scratch_instances: Vec::new(),
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
            skinned_models: Vec::new(),
            skin_slots: Vec::new(),
            skinned_draws: Vec::new(),
            skin_scratch: Vec::new(),
        };
        renderer.post.push(Box::new(BloomPass::default()));
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
        );
        self.cascade_slots =
            make_cascade_slots(&self.device, &self.shadow_layout, settings.cascades());
        self.pick_scene_targets();
        self.pipelines = make_pipelines(
            &self.device,
            self.samples,
            self.hdr_format,
            &self.global_layout,
            &self.shadow_layout,
            &self.skin_layout,
        );
        self.attachments = None;
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
        self.pick_scene_targets();
        self.pipelines = make_pipelines(
            &self.device,
            self.samples,
            self.hdr_format,
            &self.global_layout,
            &self.shadow_layout,
            &self.skin_layout,
        );
        self.attachments = None;
    }

    /// Sets [`Renderer::hdr_format`] and [`Renderer::msaa_samples`] for the current settings.
    fn pick_scene_targets(&mut self) {
        (self.hdr_format, self.samples) = scene_targets(
            &self.device,
            self.adapter.as_ref(),
            self.hdr_features,
            self.settings,
        );
    }

    /// The MSAA sample count in use: the quality preset's count (4 for [`Quality::High`]),
    /// lowered to the highest count the scene target format and the depth format support.
    pub fn msaa_samples(&self) -> u32 {
        self.samples
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
        let samples = self.samples;
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
        att.finish_groups[i] = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("metallic finish group"),
            layout: &self.finish_gpu.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.finish_gpu.uniform.as_entire_binding(),
                },
            ],
        }));
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
                self.sky.exposure * self.finish.exposure * self.finish.curve.prescale(),
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
        self.ensure_attachments(size);
        self.upload_scene();
        // A NaN eye or field of view would reach the uniform block, the sort and the cascades.
        let camera = &camera.sanitized();
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

        let Some(att) = self.attachments.as_ref() else {
            return;
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("metallic frame"),
            });

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
            // Like Roblox, partly transparent parts cast a full shadow; invisible ones
            // (Transparency 1) are not in the blended list and cast none.
            self.draw_blended_instances(&mut pass);
            pass.set_pipeline(&self.pipelines.skinned_shadow);
            self.draw_skinned(&mut pass, None);
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
        self.post_and_finish(&mut encoder, target, camera, keep_depth);
        self.queue.submit(Some(encoder.finish()));
    }

    /// Runs the HDR passes, the finish pass and the LDR passes; the last of them writes
    /// `target`.
    fn post_and_finish(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        camera: &Camera,
        keep_depth: bool,
    ) {
        let Some(att) = self.attachments.as_ref() else {
            return;
        };
        let size = att.size;
        let samples = self.samples;
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
                camera,
            });
            cur = 1 - cur;
        }

        let ldr_count = chain.active(PostSlot::Ldr);
        let (finish_view, finish_format) = match &att.ldr[0] {
            Some(v) if ldr_count > 0 => (v, LDR_FORMAT),
            _ => (target, self.format),
        };
        let uniform = FinishUniform::new(&self.finish, self.sky.exposure, finish_format.is_srgb());
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
                camera,
            });
            cur = 1 - cur;
        }
        self.post = chain;
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
        }
    }

    fn draw_blended(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.blended.count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipelines.blended);
        self.draw_blended_instances(pass);
    }

    /// Draws the blended instances far to near with the pipeline already set (the blended
    /// pipeline, or the shadow pipeline for their shadows).
    fn draw_blended_instances(&self, pass: &mut wgpu::RenderPass<'_>) {
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
        self.render(&view, camera, size);

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
    }

    #[test]
    fn msaa_falls_back_to_the_highest_supported_count() {
        use wgpu::TextureFormatFeatureFlags as F;
        let resolve = F::MULTISAMPLE_RESOLVE;
        let all = F::MULTISAMPLE_X2 | F::MULTISAMPLE_X4 | F::MULTISAMPLE_X8 | resolve;
        assert_eq!(supported_samples(4, all, all), 4);
        assert_eq!(supported_samples(1, all, all), 1);
        // Never above the request, even when more is supported.
        assert_eq!(supported_samples(2, all, all), 2);
        // Colour without 4x: the next lower count both support.
        assert_eq!(supported_samples(4, F::MULTISAMPLE_X2 | resolve, all), 2);
        // Depth without 4x limits it too.
        assert_eq!(supported_samples(4, all, F::MULTISAMPLE_X2), 2);
        // No resolve support, or no multisampling at all: single sample.
        assert_eq!(supported_samples(4, F::MULTISAMPLE_X4, all), 1);
        assert_eq!(supported_samples(4, F::empty(), F::empty()), 1);
    }

    /// The WebGPU guarantees (used when the device has no adapter-specific format features)
    /// include 4x for the HDR, LDR and depth formats, so the High preset keeps 4x there.
    #[test]
    fn guaranteed_formats_keep_4x() {
        let g =
            |f: wgpu::TextureFormat| f.guaranteed_format_features(wgpu::Features::empty()).flags;
        for format in [HDR_FORMAT, LDR_FORMAT] {
            assert_eq!(
                supported_samples(4, g(format), g(DEPTH_FORMAT)),
                4,
                "{format:?}"
            );
        }
    }
}
