//! Drawing parts: the primitive meshes, the material table, the camera, and the draws (ADR 0008 §4).
//!
//! # A constant number of draws
//!
//! Parts are drawn with **one instanced draw per mesh kind per shading model**, so the draw count depends on how
//! many kinds and models there are, never on how many parts. There is one shading model for now (PBR-lite, below),
//! so the opaque pass makes at most one draw per mesh kind: four for the built-in primitives. The material of each
//! instance comes from the material table ([`MaterialTable`]) through the instance's material row, so materials
//! cost no draws either.
//!
//! Which slots a draw covers comes from the scene's **draw lists** ([`crate::scene`]): one list of entries per
//! mesh kind, kept current from the dirty slots alone, laid out in one buffer with a segment per kind
//! ([`crate::GpuScene::list_segments`]). A kind's draw is instances `first .. first + len` of its segment:
//!
//! - **Storage path** (every backend whose vertex shaders read storage buffers): the instance index selects an
//!   entry, and the entry names the slot whose packed instance the shader reads from the region buffers. No
//!   instance data is copied or compacted; a list entry is 4 bytes and changes only when a slot starts or stops
//!   being drawn as that kind.
//! - **Vertex-buffer path** (GLES): vertex buffers cannot be indexed through a list, so there the list holds
//!   copies of the instances, read as per-instance attributes.
//!
//! Culling ([`crate::cull`], on by default: [`crate::RendererSettings::cull`]) fills the same lists with the
//! *visible* entries of each kind instead of all of them, front to back, and the transparent pass sorts only the
//! visible see-through instances. A GPU culling path can later write the counts into indirect draw arguments.
//! Indirect draws are not used yet: with nothing on the GPU writing the counts they would only add wgpu's
//! indirect-argument validation pass.
//!
//! # Transparency
//!
//! Partly see-through instances are drawn in the transparent pass, after opaque and the sky, with the depth test
//! on and depth writes off, blended with premultiplied alpha. They are **sorted back to front per instance**
//! (by distance from the camera to the instance's centre) whenever the camera moves or one of them changes, and
//! drawn as one draw per run of consecutive instances with the same mesh, so the order is exact across mesh kinds.
//! Fully transparent instances are not drawn at all.
//!
//! # Shading
//!
//! PBR-lite: Lambert diffuse and GGX specular (height-correlated Smith visibility, Schlick Fresnel) under one
//! directional light, plus a sky/ground hemisphere ambient with a split-sum environment term, and the material's
//! emission. The light and ambient are temporary ([`SceneLight`]); `Lighting` replaces them (#81). Colours are
//! sRGB-encoded in the instance and decoded to linear light in the shader; the output is linear, encoded by an
//! sRGB target (or by the shader for other formats).
//!
//! # Camera
//!
//! [`Camera`] is `Workspace.CurrentCamera` ([`CameraReader`]): a reverse-Z projection with an infinite far plane
//! from `FieldOfView` and the target's aspect ratio, and a rotation-only view. The translation goes into the
//! frame uniform as `camera − scene origin`, so positions reach the projection relative to the camera (ADR 0008
//! §11). Without a camera nothing is drawn.
//!
//! Every pipeline (depth prepass, opaque, opaque after a prepass, transparent) is created up front, when the
//! renderer is made or its colour format or MSAA count change, never during a frame (ADR 0008 §9).

mod camera;
mod material;
mod mesh;
mod transparent;

pub use camera::{Camera, CameraReader};
pub use material::{MAX_MATERIAL_ROWS, MaterialParams, MaterialTable};
pub use mesh::{MeshData, MeshPool, MeshRange, MeshVertex};

use bytemuck::{Pod, Zeroable};
use metallic_core::glam::Vec3;

use crate::depth::{DEPTH_COMPARE, DEPTH_FORMAT, depth_stencil_state};
use crate::scene::{GpuInstance, InstanceBinding, MeshKind};
use crate::{GpuScene, PassId};
use material::MATERIAL_TABLE_SIZE;
use transparent::TransparentList;

/// The temporary scene light: one directional light and a sky/ground ambient. `Lighting` replaces it (#81).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLight {
    /// Direction towards the light (normalised when drawn).
    pub direction: Vec3,
    /// The light's radiance, linear RGB (a white surface facing it reflects about a third of this).
    pub color: Vec3,
    /// Ambient light from the sky (above), linear RGB.
    pub sky_ambient: Vec3,
    /// Ambient light from the ground (below), linear RGB.
    pub ground_ambient: Vec3,
}

impl Default for SceneLight {
    /// A sun high over the left shoulder of a camera looking down −Z, and a bluish sky.
    fn default() -> Self {
        Self {
            direction: Vec3::new(-0.4, 0.8, 0.45),
            color: Vec3::new(2.3, 2.2, 2.05),
            sky_ambient: Vec3::new(0.3, 0.34, 0.4),
            ground_ambient: Vec3::new(0.14, 0.12, 0.1),
        }
    }
}

/// What the last frame drew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawStats {
    /// Draws in the depth prepass (one per mesh kind with instances; 0 without a prepass).
    pub prepass_draws: u32,
    /// Draws in the opaque pass: one per mesh kind with instances, whatever the part count.
    pub opaque_draws: u32,
    /// Draws in the transparent pass: one per run of same-mesh instances in back-to-front order.
    pub transparent_draws: u32,
    /// Instances drawn in the opaque and transparent passes.
    pub instances: u32,
    /// Triangles drawn in the opaque and transparent passes.
    pub triangles: u64,
    /// Instances drawn in the opaque pass (and again in the depth prepass when there is one).
    pub opaque_instances: u32,
    /// Triangles drawn in the opaque pass (and again in the depth prepass when there is one).
    pub opaque_triangles: u64,
    /// Instances drawn in the transparent pass.
    pub transparent_instances: u32,
    /// Triangles drawn in the transparent pass.
    pub transparent_triangles: u64,
    /// Instances culled from the frame (outside the view, too far or too small; see [`crate::cull`]).
    pub culled_instances: u32,
}

impl DrawStats {
    /// Every draw of the frame.
    pub fn total_draws(&self) -> u32 {
        self.prepass_draws + self.opaque_draws + self.transparent_draws
    }
}

/// The frame uniform (`Frame` in `parts.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
struct FrameUniforms {
    view_proj: [[f32; 4]; 4],
    camera_offset: [f32; 4],
    light_direction: [f32; 4],
    light_color: [f32; 4],
    sky_ambient: [f32; 4],
    ground_ambient: [f32; 4],
    output: [f32; 4],
}

fn vec4(v: Vec3) -> [f32; 4] {
    [v.x, v.y, v.z, 0.0]
}

/// One opaque draw: a mesh kind's index range over its list segment.
#[derive(Clone, Copy, Debug, Default)]
struct KindDraw {
    mesh: MeshRange,
    first: u32,
    count: u32,
}

/// The render pipelines for one colour format and sample count.
#[derive(Debug)]
struct Pipelines {
    format: wgpu::TextureFormat,
    samples: u32,
    prepass: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    /// Opaque after a depth prepass: tests depth (`GreaterEqual`) without writing it.
    opaque_tested: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
}

/// The storage path's instance bind group and the buffers it was made for.
#[derive(Debug)]
struct InstanceGroup {
    buffers: [wgpu::Buffer; 3],
    group: wgpu::BindGroup,
}

/// Draws a [`GpuScene`]'s parts into the renderer's passes. [`crate::Renderer`] owns one; see the
/// [module documentation](self) for the design.
#[derive(Debug)]
pub struct PartRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    binding: InstanceBinding,
    meshes: MeshPool,
    materials: MaterialTable,
    light: SceneLight,
    frame_buffer: wgpu::Buffer,
    /// What `frame_buffer` holds, so an unchanged frame skips the write.
    frame_uniforms: Option<FrameUniforms>,
    frame_group: wgpu::BindGroup,
    instance_layout: Option<wgpu::BindGroupLayout>,
    pipeline_layout: wgpu::PipelineLayout,
    module: wgpu::ShaderModule,
    pipelines: Pipelines,
    opaque_group: Option<InstanceGroup>,
    transparent_group: Option<InstanceGroup>,
    transparent: TransparentList,
    draws: Vec<KindDraw>,
    /// Whether this frame has a camera (and so draws anything).
    visible: bool,
    stats: DrawStats,
}

impl PartRenderer {
    /// Creates the meshes, the material table and every pipeline for targets in `format` with `samples` samples,
    /// on the instance path `binding` (which must be the one the scenes it draws use:
    /// [`InstanceBinding::for_context`]).
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        binding: InstanceBinding,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let source = match binding {
            InstanceBinding::Storage => concat!(include_str!("parts.wgsl"), include_str!("instance_storage.wgsl")),
            InstanceBinding::Vertex => concat!(include_str!("parts.wgsl"), include_str!("instance_vertex.wgsl")),
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("metallic parts"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let uniform = |binding, visibility, size: u64| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic parts frame"),
            entries: &[
                uniform(
                    0,
                    wgpu::ShaderStages::VERTEX_FRAGMENT,
                    size_of::<FrameUniforms>() as u64,
                ),
                uniform(1, wgpu::ShaderStages::FRAGMENT, MATERIAL_TABLE_SIZE),
            ],
        });
        let instance_layout = (binding == InstanceBinding::Storage).then(|| {
            let storage = |binding, size: u64| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size),
                },
                count: None,
            };
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("metallic parts instances"),
                entries: &[
                    storage(0, GpuInstance::SIZE),
                    storage(1, GpuInstance::SIZE),
                    storage(2, size_of::<u32>() as u64),
                ],
            })
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("metallic parts"),
            bind_group_layouts: &[Some(&frame_layout), instance_layout.as_ref()],
            immediate_size: 0,
        });
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic parts frame"),
            size: size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let materials = MaterialTable::new(device);
        debug_assert_eq!(materials.buffer().size(), MATERIAL_TABLE_SIZE);
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("metallic parts frame"),
            layout: &frame_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: materials.buffer().as_entire_binding(),
                },
            ],
        });
        let pipelines = create_pipelines(device, &pipeline_layout, &module, binding, format, samples);
        Self {
            device: device.clone(),
            queue: queue.clone(),
            binding,
            meshes: MeshPool::new(device, queue),
            materials,
            light: SceneLight::default(),
            frame_buffer,
            frame_uniforms: None,
            frame_group,
            instance_layout,
            pipeline_layout,
            module,
            pipelines,
            opaque_group: None,
            transparent_group: None,
            transparent: TransparentList::new(device, binding),
            draws: Vec::new(),
            visible: false,
            stats: DrawStats::default(),
        }
    }

    /// The instance path the pipelines were made for.
    pub fn binding(&self) -> InstanceBinding {
        self.binding
    }

    /// Recreates the pipelines for another colour format or sample count (a settings change, never per frame).
    pub(crate) fn set_target(&mut self, format: wgpu::TextureFormat, samples: u32) {
        if (format, samples) != (self.pipelines.format, self.pipelines.samples) {
            self.pipelines = create_pipelines(
                &self.device,
                &self.pipeline_layout,
                &self.module,
                self.binding,
                format,
                samples,
            );
        }
    }

    /// The material table. Rows changed here are uploaded with the next frame.
    pub fn materials_mut(&mut self) -> &mut MaterialTable {
        &mut self.materials
    }

    /// The material table.
    pub fn materials(&self) -> &MaterialTable {
        &self.materials
    }

    /// The temporary light.
    pub fn light(&self) -> SceneLight {
        self.light
    }

    /// Changes the temporary light.
    pub fn set_light(&mut self, light: SceneLight) {
        self.light = light;
    }

    /// The meshes.
    pub fn meshes(&self) -> &MeshPool {
        &self.meshes
    }

    /// What the last frame drew.
    pub fn stats(&self) -> DrawStats {
        self.stats
    }

    /// Prepares a frame of `scene` seen from `camera` on a `width` × `height` target: the uniforms, the material
    /// table, the sorted transparent list and the bind groups. Call it after [`GpuScene::upload_view`] (which may
    /// replace buffers, and decides what is drawn) and before the passes. `None` draws nothing.
    pub(crate) fn prepare(&mut self, scene: &GpuScene, camera: Option<&Camera>, width: u32, height: u32, srgb: bool) {
        self.stats = DrawStats::default();
        self.draws.clear();
        self.visible = false;
        let Some(camera) = camera else {
            return;
        };
        if scene.binding() != self.binding {
            // A scene made for another device; never happens with one context.
            return;
        }
        self.visible = true;
        let instances = scene.scene();
        let offset = camera.position - instances.origin();

        self.materials.fill_from_names(instances.material_names());
        self.materials.upload(&self.queue);
        let light = &self.light;
        let uniforms = FrameUniforms {
            view_proj: camera
                .view_projection(width as f32 / height.max(1) as f32)
                .to_cols_array_2d(),
            camera_offset: vec4(offset),
            light_direction: vec4(light.direction.try_normalize().unwrap_or(Vec3::Y)),
            light_color: vec4(light.color),
            sky_ambient: vec4(light.sky_ambient),
            ground_ambient: vec4(light.ground_ambient),
            output: [if srgb { 0.0 } else { 1.0 }, 0.0, 0.0, 0.0],
        };
        if self.frame_uniforms != Some(uniforms) {
            self.queue
                .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniforms));
            self.frame_uniforms = Some(uniforms);
        }

        for (kind, segment) in scene.list_segments().iter().enumerate() {
            let mesh = self.meshes.range(MeshKind(u16::try_from(kind).unwrap_or(u16::MAX)));
            if segment.len == 0 || mesh.index_count == 0 {
                continue;
            }
            self.draws.push(KindDraw {
                mesh,
                first: segment.first,
                count: segment.len,
            });
            self.stats.opaque_instances += segment.len;
            self.stats.opaque_triangles += u64::from(segment.len) * u64::from(mesh.index_count / 3);
        }
        self.transparent.update(
            &self.device,
            &self.queue,
            instances,
            scene.transparent_entries(),
            offset,
        );
        self.stats.culled_instances = scene.cull_stats().culled();
        for run in self.transparent.runs() {
            let mesh = self.meshes.range(run.kind);
            self.stats.transparent_instances += run.count;
            self.stats.transparent_triangles += u64::from(run.count) * u64::from(mesh.index_count / 3);
        }
        self.stats.instances = self.stats.opaque_instances + self.stats.transparent_instances;
        self.stats.triangles = self.stats.opaque_triangles + self.stats.transparent_triangles;

        if let Some(layout) = self.instance_layout.as_ref() {
            let statics = scene.buffer(crate::scene::Region::Static);
            let dynamics = scene.buffer(crate::scene::Region::Dynamic);
            refresh_group(
                &self.device,
                layout,
                &mut self.opaque_group,
                [statics, dynamics, scene.list_buffer()],
            );
            refresh_group(
                &self.device,
                layout,
                &mut self.transparent_group,
                [statics, dynamics, self.transparent.buffer()],
            );
        }
    }

    /// Records the draws of `pass` (only the prepass, opaque and transparent passes draw parts). `prepass` says
    /// whether the frame has a depth prepass, so the opaque pass only tests depth.
    pub(crate) fn draw(
        &mut self,
        pass: PassId,
        prepass: bool,
        scene: &GpuScene,
        render_pass: &mut wgpu::RenderPass<'_>,
    ) {
        if !self.visible {
            return;
        }
        match pass {
            PassId::DepthPrepass | PassId::Opaque => {
                if self.draws.is_empty() {
                    return;
                }
                let pipeline = match (pass, prepass) {
                    (PassId::DepthPrepass, _) => &self.pipelines.prepass,
                    (_, true) => &self.pipelines.opaque_tested,
                    (_, false) => &self.pipelines.opaque,
                };
                render_pass.set_pipeline(pipeline);
                self.bind(render_pass, self.opaque_group.as_ref(), scene.list_buffer());
                for draw in &self.draws {
                    let indices = draw.mesh.first_index..draw.mesh.first_index + draw.mesh.index_count;
                    render_pass.draw_indexed(indices, 0, draw.first..draw.first + draw.count);
                }
                if pass == PassId::DepthPrepass {
                    self.stats.prepass_draws += self.draws.len() as u32;
                } else {
                    self.stats.opaque_draws += self.draws.len() as u32;
                }
            }
            PassId::Transparent => {
                if self.transparent.runs().is_empty() {
                    return;
                }
                render_pass.set_pipeline(&self.pipelines.transparent);
                self.bind(render_pass, self.transparent_group.as_ref(), self.transparent.buffer());
                for run in self.transparent.runs() {
                    let mesh = self.meshes.range(run.kind);
                    if mesh.index_count == 0 {
                        continue;
                    }
                    let indices = mesh.first_index..mesh.first_index + mesh.index_count;
                    render_pass.draw_indexed(indices, 0, run.first..run.first + run.count);
                    self.stats.transparent_draws += 1;
                }
            }
            PassId::Sky | PassId::Post | PassId::Ui => {}
        }
    }

    /// Binds the frame group, the meshes and the instances: the storage path's instance group, or `copies` as the
    /// per-instance vertex buffer.
    fn bind(&self, render_pass: &mut wgpu::RenderPass<'_>, group: Option<&InstanceGroup>, copies: &wgpu::Buffer) {
        render_pass.set_bind_group(0, &self.frame_group, &[]);
        render_pass.set_vertex_buffer(0, self.meshes.vertices().slice(..));
        render_pass.set_index_buffer(self.meshes.indices().slice(..), wgpu::IndexFormat::Uint32);
        match self.binding {
            InstanceBinding::Storage => {
                if let Some(group) = group {
                    render_pass.set_bind_group(1, &group.group, &[]);
                }
            }
            InstanceBinding::Vertex => render_pass.set_vertex_buffer(1, copies.slice(..)),
        }
    }
}

/// Makes `slot`'s bind group again when any of `buffers` is not the one it was made with.
fn refresh_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    slot: &mut Option<InstanceGroup>,
    buffers: [&wgpu::Buffer; 3],
) {
    if slot
        .as_ref()
        .is_some_and(|g| g.buffers.iter().zip(buffers).all(|(a, b)| a == b))
    {
        return;
    }
    fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: buffer.as_entire_binding(),
        }
    }
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("metallic parts instances"),
        layout,
        entries: &[entry(0, buffers[0]), entry(1, buffers[1]), entry(2, buffers[2])],
    });
    *slot = Some(InstanceGroup {
        buffers: buffers.map(Clone::clone),
        group,
    });
}

fn create_pipelines(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    binding: InstanceBinding,
    format: wgpu::TextureFormat,
    samples: u32,
) -> Pipelines {
    let instance_attributes = GpuInstance::vertex_attributes(2);
    let all_buffers = [
        Some(MeshVertex::layout()),
        Some(wgpu::VertexBufferLayout {
            array_stride: GpuInstance::SIZE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &instance_attributes,
        }),
    ];
    // The storage path reads instances from storage buffers, not from a second vertex buffer.
    let buffers = match binding {
        InstanceBinding::Storage => &all_buffers[..1],
        InstanceBinding::Vertex => &all_buffers[..],
    };
    let pipeline = |label: &str, depth: wgpu::DepthStencilState, fragment: Option<(&str, Option<wgpu::BlendState>)>| {
        let targets = [Some(wgpu::ColorTargetState {
            format,
            blend: fragment.and_then(|(_, blend)| blend),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: Some(depth),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..wgpu::MultisampleState::default()
            },
            fragment: fragment.map(|(entry_point, _)| wgpu::FragmentState {
                module,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let tested = wgpu::DepthStencilState {
        depth_write_enabled: Some(false),
        depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
        ..depth_stencil_state(false)
    };
    debug_assert_eq!(depth_stencil_state(true).depth_compare, Some(DEPTH_COMPARE));
    debug_assert_eq!(depth_stencil_state(true).format, DEPTH_FORMAT);
    Pipelines {
        format,
        samples,
        prepass: pipeline("metallic parts depth prepass", depth_stencil_state(true), None),
        opaque: pipeline(
            "metallic parts opaque",
            depth_stencil_state(true),
            Some(("fs_opaque", None)),
        ),
        opaque_tested: pipeline(
            "metallic parts opaque (after prepass)",
            tested,
            Some(("fs_opaque", None)),
        ),
        transparent: pipeline(
            "metallic parts transparent",
            depth_stencil_state(false),
            Some(("fs_transparent", Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING))),
        ),
    }
}
