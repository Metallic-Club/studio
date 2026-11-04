//! MLAA's edges and blend weights ([`crate::PassId::Mlaa`]): one full-screen pass at the scene's resolution from the
//! scene colour into an RGBA8 texture of weights, which the finish pass blends with (`mlaa.wgsl`).

use super::{MlaaSettings, full_screen_pipeline, post_texture, texture_entry, uniform_entry};

const PARAMS_SIZE: u64 = 32;

/// The weights' format: four areas from 0 to 0.5.
const WEIGHTS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// MLAA's pipeline (made with the renderer), its parameters, and its weights while it runs.
pub(crate) struct Mlaa {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    written: Option<[f32; 8]>,
    /// The weights texture's view and the pass's bind group, while MLAA runs.
    target: Option<(wgpu::TextureView, wgpu::BindGroup)>,
}

impl Mlaa {
    /// The pipeline.
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("metallic mlaa"),
            source: wgpu::ShaderSource::Wgsl(include_str!("mlaa.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic mlaa"),
            entries: &[uniform_entry(0, PARAMS_SIZE), texture_entry(1, false)],
        });
        let pipeline = full_screen_pipeline(
            device,
            "metallic mlaa",
            &layout,
            &module,
            ("vs_mlaa", "fs_mlaa"),
            WEIGHTS_FORMAT,
            None,
        );
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic mlaa"),
            size: PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline,
            params,
            written: None,
            target: None,
        }
    }

    /// Makes the weights for a scene colour of `size` read through `scene`, or drops them when MLAA does not run
    /// (`None`). A settings change, never per frame.
    pub(crate) fn rebuild(&mut self, device: &wgpu::Device, scene: Option<&wgpu::TextureView>, size: (u32, u32)) {
        self.target = scene.map(|scene| {
            let view = post_texture(device, "metallic mlaa weights", size, 1, WEIGHTS_FORMAT)
                .create_view(&wgpu::TextureViewDescriptor::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("metallic mlaa"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(scene),
                    },
                ],
            });
            (view, group)
        });
    }

    /// The weights the finish pass blends with, while MLAA runs.
    pub(crate) fn output(&self) -> Option<&wgpu::TextureView> {
        self.target.as_ref().map(|(view, _)| view)
    }

    /// Finds edges in the top-left `render` corner of the scene colour in the next frames, as `settings` say, with
    /// luma measured after `exposure`. Writes the parameters only when they change.
    pub(crate) fn prepare(&mut self, queue: &wgpu::Queue, render: (u32, u32), settings: &MlaaSettings, exposure: f32) {
        if self.target.is_none() {
            return;
        }
        let params = [
            render.0 as f32,
            render.1 as f32,
            settings.threshold.clamp(0.0, 1.0),
            settings.search.clamp(1, 64) as f32,
            exposure,
            0.0,
            0.0,
            0.0,
        ];
        if self.written != Some(params) {
            queue.write_buffer(&self.params, 0, bytemuck::cast_slice(&params));
            self.written = Some(params);
        }
    }

    /// Records the pass into `encoder`, into the `corner` of the weights when the scene is drawn into one.
    pub(crate) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
        corner: Option<(u32, u32)>,
    ) {
        let Some((view, group)) = self.target.as_ref() else {
            return;
        };
        let colors = [Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mlaa"),
            color_attachments: &colors,
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some((width, height)) = corner {
            pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
            pass.set_scissor_rect(0, 0, width, height);
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}
