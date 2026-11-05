//! The upscale pass ([`crate::PassId::Upscale`]): the scene colour, drawn at the render scale, stretched over the
//! whole output with one bilinear tap per pixel.
//!
//! It runs only in frames without the finish pass ([`crate::PassId::Post`]), which otherwise upscales as it finishes
//! the image (#85): one pass fewer over the whole output. A better upscaler (FSR 1 class) replaces the bilinear tap
//! here and in `post/finish.wgsl` (`scene_uv`), and nothing else. The pipeline is compiled off the frame
//! ([`crate::pipelines`], ADR 0008 §9); a frame binds it, writes its 16-byte parameters only when the render size
//! changed, and draws one triangle.

use crate::context::DescriptorHeapTurn;
use crate::post::FullScreen;

/// The upscale pass's pipeline, sampler and parameters, and the bind group of the scene colour it reads.
pub(crate) struct Upscaler {
    module: wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    pipeline: Option<wgpu::RenderPipeline>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    /// The scene colour's bind group, while there is a scene colour.
    group: Option<wgpu::BindGroup>,
    /// The parameters last written: the drawn corner's share of the texture and the farthest coordinate read.
    written: Option<[f32; 4]>,
}

impl Upscaler {
    /// The pass for outputs in `format` (the scene colour has the same format).
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("metallic upscale"),
            source: wgpu::ShaderSource::Wgsl(include_str!("upscale.wgsl").into()),
        });
        let layout_turn = DescriptorHeapTurn::take();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic upscale"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        drop(layout_turn);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("metallic upscale"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..wgpu::SamplerDescriptor::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic upscale"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            module,
            format,
            pipeline: None,
            layout,
            sampler,
            params,
            group: None,
            written: None,
        }
    }

    /// What the pipeline is made from.
    pub(crate) fn full_screen(&self) -> FullScreen {
        FullScreen {
            label: "metallic upscale",
            layout: self.layout.clone(),
            module: self.module.clone(),
            entry_points: ("vs_upscale", "fs_upscale"),
            format: self.format,
            blend: None,
        }
    }

    /// Puts the compiled pipeline in place.
    pub(crate) fn install(&mut self, pipeline: wgpu::RenderPipeline) {
        self.pipeline = Some(pipeline);
    }

    /// Whether the pipeline is compiled.
    pub(crate) fn has(&self) -> bool {
        self.pipeline.is_some()
    }

    /// Reads `source` (the scene colour) from now on, or nothing. A settings change, never per frame.
    pub(crate) fn set_source(&mut self, device: &wgpu::Device, source: Option<&wgpu::TextureView>) {
        self.group = source.map(|view| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("metallic upscale"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        });
    }

    /// Reads the top-left `render` corner of a `texture`-sized scene colour in the next frames. Writes the
    /// parameters only when they change.
    pub(crate) fn prepare(&mut self, queue: &wgpu::Queue, render: (u32, u32), texture: (u32, u32)) {
        let params = upscale_params(render, texture);
        if self.written != Some(params) {
            queue.write_buffer(&self.params, 0, bytemuck::cast_slice(&params));
            self.written = Some(params);
        }
    }

    /// Records the upscale into `pass`: one triangle over the output. Nothing without a scene colour.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(group) = self.group.as_ref()
            && let Some(pipeline) = self.pipeline.as_ref()
        {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// The upscale's parameters: the drawn `render` corner's share of the `texture`, and the farthest texture coordinate
/// read, half a texel inside the corner (so the bilinear filter never mixes in a texel outside it).
pub(crate) fn upscale_params(render: (u32, u32), texture: (u32, u32)) -> [f32; 4] {
    let (tw, th) = (texture.0.max(1) as f32, texture.1.max(1) as f32);
    let (rw, rh) = (render.0.min(texture.0) as f32, render.1.min(texture.1) as f32);
    [rw / tw, rh / th, (rw - 0.5) / tw, (rh - 0.5) / th]
}

#[cfg(test)]
mod tests {
    use super::upscale_params;

    #[test]
    fn the_whole_texture_reads_to_half_a_texel_from_its_edge() {
        assert_eq!(upscale_params((100, 50), (100, 50)), [1.0, 1.0, 0.995, 0.99]);
        assert_eq!(upscale_params((50, 25), (100, 50)), [0.5, 0.5, 0.495, 0.49]);
        // A render size larger than the texture reads all of it, no more.
        assert_eq!(upscale_params((200, 100), (100, 50)), [1.0, 1.0, 0.995, 0.99]);
    }
}
