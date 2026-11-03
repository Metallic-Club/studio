//! The frame loop: runs the planned passes into a target and keeps the GPU timings coming.

use crate::depth::{DEPTH_CLEAR, DEPTH_FORMAT};
use crate::graph::{AttachmentOps, ColorAttachment, DepthOps, FramePlan, PassSet};
use crate::target::check_size;
use crate::timing::GpuTimer;
use crate::{FrameTimings, MsaaSampleCounts, RenderContext, RenderError};

/// What a [`Renderer`] draws into and how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererSettings {
    /// The format of the targets passed to [`Renderer::render`] (the surface's or the offscreen target's).
    pub color_format: wgpu::TextureFormat,
    /// Target width in pixels.
    pub width: u32,
    /// Target height in pixels.
    pub height: u32,
    /// Samples per pixel for the scene (1: no MSAA). Must be in [`RenderContext::msaa_sample_counts`] for both the
    /// colour format and the depth format.
    pub msaa_samples: u32,
    /// Which passes run.
    pub passes: PassSet,
    /// Measure each pass's GPU time where the device has timestamp queries.
    pub gpu_timing: bool,
    /// The colour the frame starts from (linear).
    pub clear_color: wgpu::Color,
}

impl RendererSettings {
    /// Settings for a `width` × `height` target in `color_format`: no MSAA, the default passes, GPU timing on and
    /// a black clear colour.
    pub fn new(color_format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        Self {
            color_format,
            width,
            height,
            msaa_samples: 1,
            passes: PassSet::default(),
            gpu_timing: true,
            clear_color: wgpu::Color::BLACK,
        }
    }
}

/// An internal attachment (depth, multisampled colour). The view keeps its texture alive.
struct Attachment {
    view: wgpu::TextureView,
}

/// Draws frames: the passes of a [`FramePlan`] into a target view, with per-pass GPU timing.
///
/// A frame allocates nothing on the heap in this crate and never waits for the GPU (ADR 0008 §9). The passes have
/// no draws yet; the scene, sky, post and UI systems record into them as they arrive.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    settings: RendererSettings,
    plan: FramePlan,
    /// MSAA counts both the colour format and depth support.
    msaa_counts: MsaaSampleCounts,
    max_texture_size: u32,
    transient_attachments: bool,
    depth: Option<Attachment>,
    msaa_color: Option<Attachment>,
    timer: Option<GpuTimer>,
    frame: u64,
}

impl Renderer {
    /// Creates a renderer for targets described by `settings`.
    pub fn new(context: &RenderContext, settings: RendererSettings) -> Result<Self, RenderError> {
        check_size(context, settings.width, settings.height)?;
        let msaa_counts = context
            .msaa_sample_counts(settings.color_format)
            .intersect(context.msaa_sample_counts(DEPTH_FORMAT));
        let timer = if settings.gpu_timing {
            GpuTimer::new(context.device(), context.queue())
        } else {
            None
        };
        let mut renderer = Self {
            device: context.device().clone(),
            queue: context.queue().clone(),
            settings,
            plan: FramePlan::new(settings.passes, 1),
            msaa_counts,
            max_texture_size: context.capabilities().max_texture_size,
            transient_attachments: context.capabilities().transient_attachments,
            depth: None,
            msaa_color: None,
            timer,
            frame: 0,
        };
        renderer.check_msaa(settings.msaa_samples)?;
        renderer.rebuild();
        Ok(renderer)
    }

    /// The current settings.
    pub fn settings(&self) -> &RendererSettings {
        &self.settings
    }

    /// The passes a frame runs and their attachments' load and store operations.
    pub fn plan(&self) -> &FramePlan {
        &self.plan
    }

    /// How many frames have been rendered. The next frame gets this index.
    pub fn frame_index(&self) -> u64 {
        self.frame
    }

    /// Whether frames are GPU-timed (asked for in the settings and supported by the device).
    pub fn gpu_timing_enabled(&self) -> bool {
        self.timer.is_some()
    }

    /// The newest per-pass GPU timings collected, usually from a frame one to three frames back. `None` until the
    /// first readback arrives, and always when GPU timing is off or unsupported.
    pub fn gpu_timings(&self) -> Option<&FrameTimings> {
        self.timer.as_ref().and_then(GpuTimer::latest)
    }

    /// Changes the target size (on window resize). Recreates the internal attachments when the size changes.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), RenderError> {
        if width == 0 || height == 0 || width > self.max_texture_size || height > self.max_texture_size {
            return Err(RenderError::InvalidSize { width, height });
        }
        if (width, height) != (self.settings.width, self.settings.height) {
            self.settings.width = width;
            self.settings.height = height;
            self.rebuild();
        }
        Ok(())
    }

    /// Changes which passes run.
    pub fn set_passes(&mut self, passes: PassSet) {
        if passes != self.settings.passes {
            self.settings.passes = passes;
            self.rebuild();
        }
    }

    /// Changes the scene's MSAA sample count (1 turns MSAA off).
    pub fn set_msaa_samples(&mut self, samples: u32) -> Result<(), RenderError> {
        self.check_msaa(samples)?;
        if samples != self.settings.msaa_samples {
            self.settings.msaa_samples = samples;
            self.rebuild();
        }
        Ok(())
    }

    /// Changes the colour the frame starts from (linear).
    pub fn set_clear_color(&mut self, color: wgpu::Color) {
        self.settings.clear_color = color;
    }

    /// Renders one frame into `target`, which must have the settings' format and size, and submits it.
    ///
    /// Never blocks on the GPU. For a window, render into [`crate::SurfaceFrame::view`] and present afterwards; the
    /// swapchain limits how far the CPU runs ahead. Offscreen nothing does, so a loop that renders many frames
    /// should pace itself with the returned submission (for example by waiting, outside the frame, for the one from
    /// two frames back), or wgpu's in-flight frames and command-buffer pools keep growing.
    pub fn render(&mut self, target: &wgpu::TextureView) -> wgpu::SubmissionIndex {
        let slot = match self.timer.as_mut() {
            Some(timer) => timer.begin_frame(&self.device),
            None => None,
        };
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic frame"),
        });
        let clear_color = self.settings.clear_color;
        let msaa_view = self.msaa_color.as_ref().map(|attachment| &attachment.view);
        let depth_view = self.depth.as_ref().map(|attachment| &attachment.view);

        for (index, pass) in self.plan.passes().iter().enumerate() {
            let color = pass.color.and_then(|color| match color {
                ColorAttachment::Output(ops) => Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: color_ops(ops, clear_color),
                }),
                ColorAttachment::Msaa { ops, resolve } => msaa_view.map(|view| wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: resolve.then_some(target),
                    ops: color_ops(ops, clear_color),
                }),
            });
            let colors = [color];
            let depth = pass.depth.and_then(|depth| {
                depth_view.map(|view| wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: match depth {
                        DepthOps::Write(ops) => Some(wgpu::Operations {
                            load: if ops.clear {
                                wgpu::LoadOp::Clear(DEPTH_CLEAR)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: store_op(ops),
                        }),
                        DepthOps::ReadOnly => None,
                    },
                    stencil_ops: None,
                })
            });
            let has_color = usize::from(colors[0].is_some());
            if has_color == 0 && depth.is_none() {
                // Only possible if an attachment is missing, which `rebuild` rules out.
                continue;
            }
            let timestamp_writes = match (self.timer.as_ref(), slot) {
                (Some(timer), Some(slot)) => Some(timer.pass_writes(slot, index)),
                _ => None,
            };
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(pass.id.name()),
                color_attachments: &colors[..has_color],
                depth_stencil_attachment: depth,
                timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The pass's draws go here as the systems that make them arrive.
            drop(render_pass);
        }

        if let (Some(timer), Some(slot)) = (self.timer.as_mut(), slot) {
            timer.end_frame(
                &mut encoder,
                slot,
                self.frame,
                self.plan.passes().iter().map(|pass| pass.id),
            );
        }
        let submission = self.queue.submit(std::iter::once(encoder.finish()));
        if let (Some(timer), Some(slot)) = (self.timer.as_ref(), slot) {
            timer.after_submit(slot);
        }
        self.frame += 1;
        submission
    }

    fn check_msaa(&self, samples: u32) -> Result<(), RenderError> {
        if samples == 1 || self.msaa_counts.contains(samples) {
            Ok(())
        } else {
            Err(RenderError::UnsupportedTarget {
                format: self.settings.color_format,
                samples,
            })
        }
    }

    /// Plans the frame again and recreates the attachments it needs. Runs on settings changes, never per frame.
    fn rebuild(&mut self) {
        self.plan = FramePlan::new(self.settings.passes, self.settings.msaa_samples);
        let transient = self.transient_attachments;
        self.depth = self
            .plan
            .uses_depth()
            .then(|| self.attachment("metallic depth", DEPTH_FORMAT, transient && self.plan.depth_transient()));
        self.msaa_color = self.plan.uses_msaa_color().then(|| {
            self.attachment(
                "metallic msaa colour",
                self.settings.color_format,
                transient && self.plan.msaa_color_transient(),
            )
        });
    }

    fn attachment(&self, label: &str, format: wgpu::TextureFormat, transient: bool) -> Attachment {
        let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if transient {
            // Lives only inside one pass (cleared, then discarded): tile memory only on tile-based GPUs.
            usage |= wgpu::TextureUsages::TRANSIENT_ATTACHMENT;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: self.settings.width,
                height: self.settings.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: self.plan.msaa_samples(),
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        Attachment {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }
}

fn color_ops(ops: AttachmentOps, clear: wgpu::Color) -> wgpu::Operations<wgpu::Color> {
    wgpu::Operations {
        load: if ops.clear {
            wgpu::LoadOp::Clear(clear)
        } else {
            wgpu::LoadOp::Load
        },
        store: store_op(ops),
    }
}

fn store_op(ops: AttachmentOps) -> wgpu::StoreOp {
    if ops.store {
        wgpu::StoreOp::Store
    } else {
        wgpu::StoreOp::Discard
    }
}
