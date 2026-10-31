//! Histogram auto exposure (#151): a GPU meter that sits between the HDR post passes and the
//! finish pass, separate from the tone curve.
//!
//! The lighting already pre-exposes the scene on the CPU (the sky table's exposure for the sun
//! elevation, `ExposureCompensation` and the night bias: the `Fixed` mode). With auto exposure
//! on, two compute passes after each frame build a centre-weighted 256-bin log-luminance
//! histogram of the HDR image the finish pass read, average the 30%..95% band, and turn it into
//! an EV100 that adapts over time. The result is a multiplier on top of the CPU pre-exposure,
//! which the next frame copies into its uniforms (the finish pass and toon shading read it), so
//! metering never reads back to the CPU. `docs/research/2026-10-30-lighting.md` 3.9 and 5.5.
/// Lowest histogram bin edge, log2 of pre-exposed luminance.
const MIN_LOG2: f32 = -12.0;
/// Highest histogram bin edge.
const MAX_LOG2: f32 = 8.0;
/// EV100 the default lighting's fixed exposure meters as (a sunny day).
pub const REFERENCE_EV100: f32 = 15.0;
/// Pre-exposed log2 luminance the meter maps to the image's mid-tone. Measured: the
/// look-presets reference scene under the default lighting meters to this with the fixed
/// exposure, so auto exposure leaves a typical daylight scene where the fixed exposure put it.
pub const METER_KEY_LOG2: f32 = -2.22;

/// How a Look sets exposure (`docs/design/look-presets.md` 3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExposureMode {
    /// Histogram metering on top of the fixed exposure (`Lighting.AutoExposure` can still turn
    /// it off).
    #[default]
    Auto,
    /// The fixed exposure from the sun elevation only: palette Looks must not drift.
    Fixed,
    /// No metering and no curve pre-scale: scene 1.0 is display 1.0 (2D Looks).
    Display,
}

/// Auto exposure settings; [`crate::Renderer::set_lighting`] fills them from the `Lighting`
/// service (`AutoExposure`, `ExposureMin`, `ExposureMax`, `ExposureCompensation`,
/// `NightAdaptation`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoExposure {
    /// Meter the image; off keeps the fixed (CPU) exposure.
    pub enabled: bool,
    /// Lowest metered EV100 (the darkest scene that still gets brighter).
    pub min_ev100: f32,
    /// Highest metered EV100 (the brightest scene that still gets darker).
    pub max_ev100: f32,
    /// EV added after metering.
    pub compensation: f32,
    /// 0..1: strength of the night bias.
    pub night_adaptation: f32,
    /// Adaptation rate toward brighter scenes, 1/s.
    pub speed_up: f32,
    /// Adaptation rate toward darker scenes, 1/s.
    pub speed_down: f32,
}

impl Default for AutoExposure {
    fn default() -> Self {
        AutoExposure {
            enabled: true,
            min_ev100: -4.0,
            max_ev100: 16.0,
            compensation: 0.0,
            night_adaptation: 1.0,
            speed_up: 3.0,
            speed_down: 1.0,
        }
    }
}

/// What the meter last wrote, read back for tests and tools ([`crate::Renderer::read_exposure`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ExposureState {
    /// Multiplier on top of the fixed exposure (1 = the fixed exposure).
    pub multiplier: f32,
    /// Adapted EV100.
    pub ev100: f32,
    /// Metered log2 luminance of the pre-exposed image.
    pub metered_log2: f32,
    /// EV100 before adaptation.
    pub target_ev100: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    min_log2: f32,
    inv_log2_range: f32,
    log2_base: f32,
    ev_offset: f32,
    log2_key: f32,
    delta_time: f32,
    ev_min: f32,
    ev_max: f32,
    compensation: f32,
    night_adaptation: f32,
    speed_up: f32,
    speed_down: f32,
    low_percent: f32,
    high_percent: f32,
    reset: u32,
    log2_scale: f32,
}

/// Whether `device` can run the meter (compute shaders and storage buffers; not WebGL2).
pub(crate) fn supported(device: &wgpu::Device) -> bool {
    let l = device.limits();
    l.max_compute_invocations_per_workgroup >= 256
        && l.max_compute_workgroup_size_x >= 256
        && l.max_storage_buffers_per_shader_stage >= 2
}

pub(crate) const EXPOSURE_WGSL: &str = include_str!("../shaders/exposure.wgsl");

/// The meter's GPU objects.
pub(crate) struct ExposureGpu {
    layout: wgpu::BindGroupLayout,
    build: wgpu::ComputePipeline,
    resolve: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    histogram: wgpu::Buffer,
    /// [`ExposureState`]; `multiplier` (offset 0) is copied into the frame uniforms.
    pub state: wgpu::Buffer,
    /// Bind groups per HDR target index, for the targets generation in `groups_targets`.
    groups: [Option<wgpu::BindGroup>; 2],
    groups_targets: u64,
}

impl ExposureGpu {
    pub(crate) fn new(device: &wgpu::Device) -> ExposureGpu {
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
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("metallic exposure"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(2),
                storage(3),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("metallic exposure"),
            source: wgpu::ShaderSource::Wgsl(EXPOSURE_WGSL.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("metallic exposure"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        use wgpu::BufferUsages as U;
        ExposureGpu {
            build: pipeline("build_histogram"),
            resolve: pipeline("resolve_exposure"),
            layout,
            params: buffer(
                "metallic exposure params",
                std::mem::size_of::<Params>() as u64,
                U::UNIFORM | U::COPY_DST,
            ),
            // Zeroed at creation; the resolve pass clears it after every use.
            histogram: buffer("metallic exposure histogram", 256 * 4, U::STORAGE),
            // Zeroed: multiplier 0 means "no previous frame" to the shader.
            state: buffer(
                "metallic exposure state",
                std::mem::size_of::<ExposureState>() as u64,
                U::STORAGE | U::COPY_SRC | U::COPY_DST,
            ),
            groups: [None, None],
            groups_targets: u64::MAX,
        }
    }

    /// Records the two compute passes on `hdr` (HDR target `index` of targets generation
    /// `targets`). The image is pre-exposed by `base_exposure` (the lighting's fixed exposure)
    /// and the finish multiplies it by `image_scale` before the curve (sky and finish exposure,
    /// not the curve pre-scale); the multiplier comes on top of both.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::TextureView,
        index: usize,
        targets: u64,
        size: (u32, u32),
        settings: &AutoExposure,
        base_exposure: f32,
        image_scale: f32,
        reference_exposure: f32,
        dt: f32,
        reset: bool,
    ) {
        if self.groups_targets != targets {
            self.groups = [None, None];
            self.groups_targets = targets;
        }
        let group = self.groups[index].get_or_insert_with(|| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("metallic exposure"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(hdr),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.histogram.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.state.as_entire_binding(),
                    },
                ],
            })
        });
        let finite = |x: f32, d: f32| if x.is_finite() { x } else { d };
        let base = finite(base_exposure, 1.0).max(1e-20);
        let scale = finite(image_scale, 1.0).max(1e-20);
        let reference = finite(reference_exposure, 1.0).max(1e-20);
        let min_ev = finite(settings.min_ev100, -4.0);
        let max_ev = finite(settings.max_ev100, 16.0).max(min_ev);
        let params = Params {
            min_log2: MIN_LOG2,
            inv_log2_range: 1.0 / (MAX_LOG2 - MIN_LOG2),
            log2_base: base.log2() + scale.log2(),
            ev_offset: REFERENCE_EV100 + reference.log2(),
            log2_key: METER_KEY_LOG2,
            delta_time: finite(dt, 0.0).clamp(0.0, 1.0),
            ev_min: min_ev,
            ev_max: max_ev,
            compensation: finite(settings.compensation, 0.0).clamp(-10.0, 10.0),
            night_adaptation: finite(settings.night_adaptation, 1.0).clamp(0.0, 1.0),
            speed_up: finite(settings.speed_up, 3.0).max(0.0),
            speed_down: finite(settings.speed_down, 1.0).max(0.0),
            low_percent: 0.30,
            high_percent: 0.95,
            reset: u32::from(reset),
            log2_scale: scale.log2(),
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("metallic exposure"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &*group, &[]);
        pass.set_pipeline(&self.build);
        pass.dispatch_workgroups(size.0.div_ceil(16), size.1.div_ceil(16), 1);
        pass.set_pipeline(&self.resolve);
        pass.dispatch_workgroups(1, 1, 1);
    }

    /// Reads the state back (blocking; tests and tools).
    pub(crate) fn read(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<ExposureState> {
        let size = std::mem::size_of::<ExposureState>() as u64;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic exposure readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic exposure readback"),
        });
        encoder.copy_buffer_to_buffer(&self.state, 0, &staging, 0, size);
        queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        rx.recv().ok()?.ok()?;
        let state = {
            let mapped = slice.get_mapped_range().ok()?;
            *bytemuck::from_bytes::<ExposureState>(&mapped)
        };
        staging.unmap();
        Some(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_shader_validates() {
        let module = naga::front::wgsl::parse_str(EXPOSURE_WGSL)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(EXPOSURE_WGSL)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(EXPOSURE_WGSL)));
    }

    #[test]
    fn layouts_match_the_shader() {
        assert_eq!(std::mem::size_of::<Params>(), 64);
        assert_eq!(std::mem::size_of::<ExposureState>(), 16);
    }
}
