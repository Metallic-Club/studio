//! The finish pass ([`crate::PassId::Post`]): the HDR scene colour into the output, with bloom, MLAA's blend and the
//! render scale's upscale fused in (`finish.wgsl`).

use super::{FullScreen, sampler_entry, texture_entry, uniform_entry};

/// The tone curve the finish pass applies to each channel after exposure (`tone` in `finish.wgsl`): the identity up
/// to 0.85, then a shoulder with the same slope there that approaches 1, so values the output used to clip keep a
/// little detail. 1.0 maps to 0.945, 1.5 to 0.996.
pub fn tone_curve(x: f32) -> f32 {
    const KNEE: f32 = 0.85;
    if x <= KNEE {
        x.max(0.0)
    } else {
        KNEE + (1.0 - KNEE) * (1.0 - (-(x - KNEE) / (1.0 - KNEE)).exp())
    }
}

/// The finish pass's parameters (`Finish` in `finish.wgsl`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FinishParams {
    /// The drawn corner's share of the scene texture, and the farthest coordinate read.
    pub(crate) rect: [f32; 4],
    /// Exposure, bloom's share, 1 to encode sRGB in the shader, the dither's amplitude.
    pub(crate) params: [f32; 4],
    /// The colour filter, and bloom's scale (one over its levels).
    pub(crate) tint: [f32; 4],
    /// Saturation and contrast.
    pub(crate) grade: [f32; 4],
    /// One scene texel in texture coordinates, and the drawn corner's size in texels.
    pub(crate) texel: [f32; 4],
}

impl FinishParams {
    fn words(&self) -> [f32; 20] {
        let mut words = [0.0; 20];
        for (chunk, row) in
            words
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip([self.rect, self.params, self.tint, self.grade, self.texel])
        {
            chunk.copy_from_slice(&row);
        }
        words
    }
}

const PARAMS_SIZE: u64 = 80;

/// The entry points of the four variants: plain, with bloom, with MLAA, with both.
const ENTRY_POINTS: [&str; 4] = ["fs_finish", "fs_finish_bloom", "fs_finish_mlaa", "fs_finish_bloom_mlaa"];

/// The finish pass: a pipeline (compiled apart, [`crate::pipelines`]) and bind group layout per variant, its
/// parameters, and the bind group of the variant in use.
pub(crate) struct Finish {
    module: wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    layouts: [wgpu::BindGroupLayout; 4],
    pipelines: [Option<wgpu::RenderPipeline>; 4],
    params: wgpu::Buffer,
    written: Option<FinishParams>,
    /// The variant in use and its bind group, while there is a scene colour.
    group: Option<(usize, wgpu::BindGroup)>,
}

impl Finish {
    /// The pass writing outputs in `format`, before any variant's pipeline is compiled.
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("metallic finish"),
            source: wgpu::ShaderSource::Wgsl(include_str!("finish.wgsl").into()),
        });
        let layouts: [wgpu::BindGroupLayout; 4] = std::array::from_fn(|variant| {
            let (bloom, mlaa) = (variant & 1 != 0, variant & 2 != 0);
            let mut entries = vec![uniform_entry(0, PARAMS_SIZE), texture_entry(1, true), sampler_entry(2)];
            if bloom {
                entries.push(texture_entry(3, true));
            }
            if mlaa {
                entries.push(texture_entry(4, false));
            }
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(ENTRY_POINTS[variant]),
                entries: &entries,
            })
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic finish"),
            size: PARAMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            module,
            format,
            layouts,
            pipelines: Default::default(),
            params,
            written: None,
            group: None,
        }
    }

    /// What variant `variant`'s pipeline is made from (bit 0: bloom, bit 1: MLAA).
    pub(crate) fn full_screen(&self, variant: usize) -> FullScreen {
        let variant = variant.min(3);
        FullScreen {
            label: ENTRY_POINTS[variant],
            layout: self.layouts[variant].clone(),
            module: self.module.clone(),
            entry_points: ("vs_finish", ENTRY_POINTS[variant]),
            format: self.format,
            blend: None,
        }
    }

    /// Puts variant `variant`'s compiled pipeline in place.
    pub(crate) fn install(&mut self, variant: usize, pipeline: wgpu::RenderPipeline) {
        if let Some(slot) = self.pipelines.get_mut(variant) {
            *slot = Some(pipeline);
        }
    }

    /// Whether variant `variant`'s pipeline is compiled.
    pub(crate) fn has(&self, variant: usize) -> bool {
        self.pipelines.get(variant).is_some_and(Option::is_some)
    }

    /// Reads `scene` from now on, with `bloom` and MLAA's `weights` when they run, or nothing. A settings change,
    /// never per frame.
    pub(crate) fn set_sources(
        &mut self,
        device: &wgpu::Device,
        sampler: &wgpu::Sampler,
        scene: Option<&wgpu::TextureView>,
        bloom: Option<&wgpu::TextureView>,
        weights: Option<&wgpu::TextureView>,
    ) {
        self.group = scene.map(|scene| {
            let variant = usize::from(bloom.is_some()) | usize::from(weights.is_some()) << 1;
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(scene),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ];
            if let Some(bloom) = bloom {
                entries.push(wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(bloom),
                });
            }
            if let Some(weights) = weights {
                entries.push(wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(weights),
                });
            }
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(ENTRY_POINTS[variant]),
                layout: &self.layouts[variant],
                entries: &entries,
            });
            (variant, group)
        });
    }

    /// Uses `params` from the next frame, writing them only when they changed.
    pub(crate) fn prepare(&mut self, queue: &wgpu::Queue, params: FinishParams) {
        if self.written != Some(params) {
            queue.write_buffer(&self.params, 0, bytemuck::cast_slice(&params.words()));
            self.written = Some(params);
        }
    }

    /// Records the pass's draw: one triangle over the output. Nothing without a scene colour.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some((variant, group)) = self.group.as_ref()
            && let Some(pipeline) = self.pipelines[*variant].as_ref()
        {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tone_curve;

    #[test]
    fn the_tone_curve_keeps_the_image_below_the_knee_and_rolls_off_above() {
        for x in [0.0, 0.1, 0.5, 0.85] {
            assert_eq!(tone_curve(x), x);
        }
        assert_eq!(tone_curve(-1.0), 0.0);
        // Smooth at the knee, below 1 everywhere above it, and rising.
        assert!((tone_curve(0.851) - 0.851).abs() < 1e-5);
        let mut previous = 0.85;
        for i in 1..20 {
            let y = tone_curve(0.85 + i as f32 * 0.05);
            assert!(y > previous && y < 1.0, "{y}");
            previous = y;
        }
        assert!(tone_curve(100.0) <= 1.0);
        assert!((tone_curve(1.0) - 0.945).abs() < 1e-3);
        assert!(tone_curve(1.5) > 0.995);
    }
}
