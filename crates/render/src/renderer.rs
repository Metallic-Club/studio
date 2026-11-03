//! The frame loop: runs the planned passes into a target and keeps the GPU timings coming.

use crate::cull::{CullSettings, CullStats, CullView};
use crate::depth::{DEPTH_CLEAR, DEPTH_FORMAT};
use crate::graph::{AttachmentOps, ColorAttachment, DepthOps, FramePlan, PassId, PassSet};
use crate::lighting::SceneLighting;
use crate::lights::{LightSettings, LightStats};
use crate::overlay::{Overlay, OverlayPass};
use crate::parts::{Camera, DrawStats, PartRenderer};
use crate::scene::{InstanceBinding, UploadStats};
use crate::shadow::{MAX_CASCADES, ShadowSettings, ShadowStats};
use crate::target::check_size;
use crate::timing::{GpuTimer, Timed};
use crate::{FrameTimings, GpuScene, MsaaSampleCounts, RenderContext, RenderError};

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
    /// What the main view culls ([`crate::cull`]): frustum, view distance and screen size.
    pub cull: CullSettings,
    /// How the sun's shadows are drawn ([`crate::shadow`]).
    pub shadows: ShadowSettings,
    /// How point and spot lights are drawn: their budget and their shadow atlas ([`crate::lights`]).
    pub lights: LightSettings,
}

impl RendererSettings {
    /// Settings for a `width` × `height` target in `color_format`: no MSAA, the default passes, GPU timing on, a
    /// black clear colour, the default culling, shadows and local lights.
    pub fn new(color_format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        Self {
            color_format,
            width,
            height,
            msaa_samples: 1,
            passes: PassSet::default(),
            gpu_timing: true,
            clear_color: wgpu::Color::BLACK,
            cull: CullSettings::default(),
            shadows: ShadowSettings::default(),
            lights: LightSettings::default(),
        }
    }
}

/// An internal attachment (depth, multisampled colour). The view keeps its texture alive.
struct Attachment {
    view: wgpu::TextureView,
}

/// Draws frames: the passes of a [`FramePlan`] into a target view, with per-pass GPU timing.
///
/// A frame allocates nothing on the heap in this crate and never waits for the GPU (ADR 0008 §9). Parts and the
/// sky are drawn by [`Renderer::render_scene`]; the post and UI systems record into the passes as they arrive.
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
    parts: PartRenderer,
    last_upload: UploadStats,
    last_cull: CullStats,
    frame: u64,
    overlay: Overlay,
    /// The overlay's GPU side; `Some` while the overlay is on.
    overlay_pass: Option<OverlayPass>,
    /// The fewest layers a shadow map array gets on this backend (2 on OpenGL).
    shadow_min_layers: u32,
    /// The shadow cascades drawn this frame, in the order their passes were recorded (their queries come first).
    shadow_passes: [u8; MAX_CASCADES],
    shadow_recorded: usize,
    /// Shadow cascades in use this frame.
    shadow_in_use: u8,
    /// The local lights' shadow atlas was drawn this frame (its query follows the cascades').
    atlas_recorded: bool,
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
        // Single-sampled until `rebuild`, after the MSAA count is checked.
        let parts = PartRenderer::new(
            context.device(),
            context.queue(),
            InstanceBinding::for_context(context),
            settings.color_format,
            1,
        );
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
            parts,
            last_upload: UploadStats::default(),
            last_cull: CullStats::default(),
            frame: 0,
            overlay: Overlay::default(),
            overlay_pass: None,
            // A one-layer texture cannot be sampled as an array there.
            shadow_min_layers: if context.capabilities().backend == wgpu::Backend::Gl {
                2
            } else {
                1
            },
            shadow_passes: [0; MAX_CASCADES],
            shadow_recorded: 0,
            shadow_in_use: 0,
            atlas_recorded: false,
        };
        renderer.check_msaa(settings.msaa_samples)?;
        renderer.rebuild();
        renderer.parts.set_shadows(settings.shadows, renderer.shadow_min_layers);
        renderer.parts.set_lights(settings.lights);
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

    /// Changes what the main view culls. Takes effect with the next frame.
    pub fn set_cull(&mut self, cull: CullSettings) {
        self.settings.cull = cull;
    }

    /// Changes how the sun's shadows are drawn ([`crate::shadow`]). A settings change, not for every frame: a new
    /// size or cascade count makes the maps again, and every map is drawn again next frame.
    pub fn set_shadows(&mut self, shadows: ShadowSettings) {
        if shadows != self.settings.shadows {
            self.settings.shadows = shadows;
            self.parts.set_shadows(shadows, self.shadow_min_layers);
        }
    }

    /// What the last [`Renderer::render_scene`] did for the sun's shadows: the cascades in use, which were drawn
    /// and which kept, their casters, and the CPU time of their culls.
    pub fn shadow_stats(&self) -> ShadowStats {
        self.parts.shadow_stats()
    }

    /// Changes how point and spot lights are drawn ([`crate::lights`]): the budget, the shadowed lights, the atlas.
    /// A settings change, not for every frame: a new atlas size makes the atlas again.
    pub fn set_lights(&mut self, lights: LightSettings) {
        if lights != self.settings.lights {
            self.settings.lights = lights;
            self.parts.set_lights(lights);
        }
    }

    /// What the last [`Renderer::render_scene`] did for point and spot lights: how many there are, are in view, in
    /// the budget, drawn and shadowed, the atlas tiles drawn, the clusters, and the CPU time.
    pub fn light_stats(&self) -> LightStats {
        self.parts.light_stats()
    }

    /// Renders one frame into `target`, which must have the settings' format and size, and submits it.
    ///
    /// Never blocks on the GPU. For a window, render into [`crate::SurfaceFrame::view`] and present afterwards; the
    /// swapchain limits how far the CPU runs ahead. Offscreen nothing does, so a loop that renders many frames
    /// should pace itself with the returned submission (for example by waiting, outside the frame, for the one from
    /// two frames back), or wgpu's in-flight frames and command-buffer pools keep growing.
    pub fn render(&mut self, target: &wgpu::TextureView) -> wgpu::SubmissionIndex {
        self.render_with(target, |_| {})
    }

    /// Like [`Renderer::render`], with `prepare` recording into the frame's encoder before the first pass: uploads
    /// the passes read, such as [`crate::GpuScene::upload`]. One submission carries both.
    pub fn render_with(
        &mut self,
        target: &wgpu::TextureView,
        prepare: impl FnOnce(&mut wgpu::CommandEncoder),
    ) -> wgpu::SubmissionIndex {
        let (mut encoder, slot) = self.begin_frame();
        prepare(&mut encoder);
        self.prepare_overlay();
        self.record_passes(&mut encoder, target, slot, None);
        self.end_frame(encoder, slot)
    }

    /// Renders one frame of `scene`'s parts, seen from `camera`, into `target` (which must have the settings'
    /// format and size), and submits it: the scene's upload ([`GpuScene::upload`]), then the passes with their
    /// draws (see [`crate::parts`]), lit by [`Renderer::lighting`] and with the sky behind them. With no camera the
    /// passes run without draws (the clear colour). Only what the camera sees is drawn, as
    /// [`RendererSettings::cull`] says ([`Renderer::cull_stats`]).
    ///
    /// Sync the scene ([`GpuScene::sync`]) first. Never blocks on the GPU; see [`Renderer::render`] about pacing.
    pub fn render_scene(
        &mut self,
        target: &wgpu::TextureView,
        scene: &mut GpuScene,
        camera: Option<&Camera>,
    ) -> wgpu::SubmissionIndex {
        let (mut encoder, slot) = self.begin_frame();
        let cull = &self.settings.cull;
        let view = camera.filter(|_| cull.enabled).map(|camera| {
            CullView::for_camera(
                camera,
                scene.scene().origin(),
                self.settings.width,
                self.settings.height,
                cull,
            )
        });
        self.last_upload = scene.upload_view(&mut encoder, view.as_ref());
        self.last_cull = scene.cull_stats();
        // After the upload (the casters are culled from what its coalesce left), before the sun's shadows take the
        // scene's caster changes.
        self.parts.prepare_lights(
            scene,
            camera,
            self.settings.width,
            self.settings.height,
            &self.settings.cull,
        );
        self.parts
            .prepare_shadows(&mut encoder, scene, camera, self.settings.width, self.settings.height);
        self.shadow_in_use = u8::try_from(self.parts.shadow_stats().cascades).unwrap_or(0);
        // After the upload, which may have replaced the buffers the bind groups use.
        self.parts.prepare(
            scene,
            camera,
            self.settings.width,
            self.settings.height,
            self.settings.color_format.is_srgb(),
        );
        self.prepare_overlay();
        self.record_shadow_passes(&mut encoder, slot);
        self.record_light_shadow_pass(&mut encoder, slot);
        self.record_passes(&mut encoder, target, slot, Some(scene));
        self.end_frame(encoder, slot)
    }

    /// Records the local lights' atlas pass when tiles are drawn this frame: one depth pass over the whole atlas,
    /// which keeps the tiles it does not draw.
    fn record_light_shadow_pass(&mut self, encoder: &mut wgpu::CommandEncoder, slot: Option<usize>) {
        let Some(view) = self.parts.light_shadow_target() else {
            return;
        };
        let timestamp_writes = match (self.timer.as_ref(), slot) {
            (Some(timer), Some(slot)) => Some(timer.pass_writes(slot, self.shadow_recorded)),
            _ => None,
        };
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("light_shadows"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.parts.draw_light_shadows(&mut render_pass);
        drop(render_pass);
        self.atlas_recorded = true;
    }

    /// Records a depth pass for every shadow cascade drawn this frame, before the frame's passes that sample them.
    fn record_shadow_passes(&mut self, encoder: &mut wgpu::CommandEncoder, slot: Option<usize>) {
        for (cascade, &label) in SHADOW_PASS_NAMES.iter().enumerate() {
            let Some(view) = self.parts.shadow_target(cascade) else {
                continue;
            };
            let timestamp_writes = match (self.timer.as_ref(), slot) {
                (Some(timer), Some(slot)) => Some(timer.pass_writes(slot, self.shadow_recorded)),
                _ => None,
            };
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.parts.draw_shadow(cascade, &mut render_pass);
            drop(render_pass);
            self.shadow_passes[self.shadow_recorded] = cascade as u8;
            self.shadow_recorded += 1;
        }
    }

    /// What the last [`Renderer::render_scene`] drew: the draw count per pass, instances and triangles.
    pub fn draw_stats(&self) -> DrawStats {
        self.parts.stats()
    }

    /// What the last [`Renderer::render_scene`] uploaded.
    pub fn upload_stats(&self) -> UploadStats {
        self.last_upload
    }

    /// What the last [`Renderer::render_scene`] culled, and the CPU time it took (default when culling is off or
    /// there was no camera).
    pub fn cull_stats(&self) -> CullStats {
        self.last_cull
    }

    /// The part renderer: the material table, the lighting, the meshes.
    pub fn parts(&self) -> &PartRenderer {
        &self.parts
    }

    /// The part renderer, to change materials or the lighting.
    pub fn parts_mut(&mut self) -> &mut PartRenderer {
        &mut self.parts
    }

    /// What [`Renderer::render_scene`] lights the scene and draws the sky with.
    pub fn lighting(&self) -> SceneLighting {
        self.parts.lighting()
    }

    /// Changes the lighting and the sky from the next frame (see [`crate::lighting`]). The engine calls it when the
    /// `Lighting` service changes; tests and tools can set any lighting without a data model.
    pub fn set_lighting(&mut self, lighting: SceneLighting) {
        self.parts.set_lighting(lighting);
    }

    /// Turns the debug overlay ([`crate::overlay`]) on or off. It is drawn in the UI pass, so only frames whose
    /// pass set has [`PassId::Ui`] show it. Turning it on the first time makes its pipeline and buffer; this is a
    /// settings change, not something to do every frame.
    pub fn set_overlay_enabled(&mut self, enabled: bool) {
        if enabled && self.overlay_pass.is_none() {
            self.overlay.reserve();
            self.overlay_pass = Some(OverlayPass::new(&self.device, self.settings.color_format));
        } else if !enabled {
            self.overlay_pass = None;
        }
    }

    /// Whether the debug overlay is drawn.
    pub fn overlay_enabled(&self) -> bool {
        self.overlay_pass.is_some()
    }

    /// What the debug overlay draws.
    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    /// What the debug overlay draws, to change it. The contents stay from frame to frame until changed; they are
    /// kept (but not drawn) while the overlay is off.
    pub fn overlay_mut(&mut self) -> &mut Overlay {
        &mut self.overlay
    }

    /// Uploads the overlay when it is on and changed.
    fn prepare_overlay(&mut self) {
        if let Some(pass) = self.overlay_pass.as_mut() {
            pass.prepare(&self.queue, &self.overlay, self.settings.width, self.settings.height);
        }
    }

    /// Starts a frame: its GPU timing slot, if any, and its encoder.
    fn begin_frame(&mut self) -> (wgpu::CommandEncoder, Option<usize>) {
        self.shadow_recorded = 0;
        self.shadow_in_use = 0;
        self.atlas_recorded = false;
        let slot = match self.timer.as_mut() {
            Some(timer) => timer.begin_frame(&self.device),
            None => None,
        };
        let encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic frame"),
        });
        (encoder, slot)
    }

    /// Records every planned pass into `encoder`, with `scene`'s draws when there is one.
    fn record_passes(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        slot: Option<usize>,
        scene: Option<&GpuScene>,
    ) {
        let clear_color = self.settings.clear_color;
        let prepass = self.settings.passes.contains(PassId::DepthPrepass);
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
                // The shadow passes recorded before took the first queries.
                (Some(timer), Some(slot)) => {
                    Some(timer.pass_writes(slot, self.shadow_recorded + usize::from(self.atlas_recorded) + index))
                }
                _ => None,
            };
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(pass.id.name()),
                color_attachments: &colors[..has_color],
                depth_stencil_attachment: depth,
                timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The post system adds its draws here when it arrives.
            if let Some(scene) = scene {
                self.parts.draw(pass.id, prepass, scene, &mut render_pass);
            }
            if pass.id == PassId::Ui
                && let Some(overlay) = self.overlay_pass.as_ref()
            {
                overlay.draw(&mut render_pass);
            }
            drop(render_pass);
        }
    }

    /// Ends the frame: resolves its timestamps, submits it and counts it.
    fn end_frame(&mut self, mut encoder: wgpu::CommandEncoder, slot: Option<usize>) -> wgpu::SubmissionIndex {
        if let (Some(timer), Some(slot)) = (self.timer.as_mut(), slot) {
            let cascades = self.shadow_passes[..self.shadow_recorded]
                .iter()
                .map(|&cascade| Timed::Cascade(cascade));
            let atlas = self.atlas_recorded.then_some(Timed::LightShadows);
            let passes = self.plan.passes().iter().map(|pass| Timed::Pass(pass.id));
            timer.end_frame(
                &mut encoder,
                slot,
                self.frame,
                cascades.chain(atlas).chain(passes),
                self.shadow_in_use,
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
        self.parts
            .set_target(self.settings.color_format, self.plan.msaa_samples());
        if let Some(overlay) = self.overlay_pass.as_mut() {
            overlay.set_format(&self.device, self.settings.color_format);
        }
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

/// The shadow passes' names in GPU debuggers.
const SHADOW_PASS_NAMES: [&str; MAX_CASCADES] = ["shadow_0", "shadow_1", "shadow_2", "shadow_3"];

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
