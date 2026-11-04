//! The frame loop: runs the planned passes into a target and keeps the GPU timings coming.

use crate::cull::{CullSettings, CullStats, CullView};
use crate::depth::{DEPTH_CLEAR, DEPTH_FORMAT};
use crate::graph::{AttachmentOps, ColorAttachment, DepthOps, FramePlan, PassId, PassSet};
use crate::lighting::SceneLighting;
use crate::lights::{LightSettings, LightStats};
use crate::lod::{LodSettings, LodView};
use crate::overlay::{Overlay, OverlayPass};
use crate::parts::{Camera, DrawStats, PartRenderer};
use crate::post::{AntiAliasing, FinishParams, Post, PostSettings, PostStats};
use crate::resolution::{DynamicResolution, ResolutionSettings, ResolutionStats, scaled_size};
use crate::scene::{InstanceBinding, UploadStats};
use crate::shadow::{MAX_CASCADES, ShadowSettings, ShadowStats};
use crate::target::check_size;
use crate::timing::{GpuTimer, Timed};
use crate::upscale::{Upscaler, upscale_params};
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
    /// Samples per pixel for the scene (1: no MSAA). Must be in [`RenderContext::msaa_sample_counts`] for the colour
    /// format, the depth format and the scene colour's ([`crate::post::hdr_format`]); [`Renderer::msaa_sample_counts`]
    /// says which are.
    pub msaa_samples: u32,
    /// Which passes run: the default draws the scene into an HDR colour target, blooms and finishes it
    /// ([`crate::post`]); MLAA is [`PassId::Mlaa`].
    pub passes: PassSet,
    /// How the post chain finishes the image: exposure, grade, bloom and MLAA ([`crate::post`]).
    pub post: PostSettings,
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
    /// How the main view chooses the round parts' levels of detail: the LOD bias ([`crate::lod`]). Levels are chosen
    /// by the cull, so with [`CullSettings::enabled`] off every part is drawn at full detail.
    pub lod: LodSettings,
    /// The scene's resolution: a render scale, or dynamic resolution holding a GPU frame time
    /// ([`crate::resolution`]). The UI is always drawn at the target's size.
    pub resolution: ResolutionSettings,
}

impl RendererSettings {
    /// Settings for a `width` × `height` target in `color_format`: no MSAA, the default passes (the scene, bloom,
    /// the finish pass and UI), GPU timing on, a black clear colour, the default post chain, culling, shadows, local
    /// lights and levels of detail, at the target's own resolution.
    pub fn new(color_format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        Self {
            color_format,
            width,
            height,
            msaa_samples: 1,
            passes: PassSet::default(),
            post: PostSettings::default(),
            gpu_timing: true,
            clear_color: wgpu::Color::BLACK,
            cull: CullSettings::default(),
            shadows: ShadowSettings::default(),
            lights: LightSettings::default(),
            lod: LodSettings::default(),
            resolution: ResolutionSettings::NATIVE,
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
    /// The scene colour, while the scene is drawn into one ([`FramePlan::uses_scene_color`]): the HDR target the
    /// finish pass reads, or without it the texture at the render scale the upscale pass reads.
    scene_color: Option<Attachment>,
    /// The scene colour's format with the post chain ([`crate::post::hdr_format`]).
    hdr_format: wgpu::TextureFormat,
    /// The post chain's pipelines and textures.
    post: Post,
    /// The size of the scene's attachments (depth, MSAA colour, scene colour): the target's, or with a render scale
    /// its highest scale's.
    texture_size: (u32, u32),
    /// The size the scene is drawn at this frame: the top-left corner of the scene's attachments.
    render_size: (u32, u32),
    resolution: DynamicResolution,
    upscaler: Upscaler,
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
    /// The shadow passes recorded this frame, in order (their queries come first): the cascades drawn and the
    /// cached cascades built.
    shadow_passes: [Timed; 2 * MAX_CASCADES],
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
        let hdr_format = crate::post::hdr_format(context);
        let msaa_counts = context
            .msaa_sample_counts(settings.color_format)
            .intersect(context.msaa_sample_counts(hdr_format))
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
            scene_color: None,
            hdr_format,
            // Every pipeline of the chain is made with the renderer: turning an effect on compiles nothing.
            post: Post::new(context.device(), settings.color_format, hdr_format),
            texture_size: (settings.width, settings.height),
            render_size: (settings.width, settings.height),
            resolution: DynamicResolution::new(settings.resolution),
            // Made with the renderer even when nothing is scaled: turning a render scale on later compiles nothing.
            upscaler: Upscaler::new(context.device(), settings.color_format),
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
            shadow_passes: [Timed::Cascade(0); 2 * MAX_CASCADES],
            shadow_recorded: 0,
            shadow_in_use: 0,
            atlas_recorded: false,
        };
        renderer.check_msaa(settings.msaa_samples)?;
        renderer.rebuild();
        // The scene's pipelines for every MSAA count the anti-aliasing settings can switch to, so a switch compiles
        // nothing (ADR 0008 §9).
        let format = renderer.scene_target_format();
        for samples in msaa_counts.iter().filter(|&n| n <= 8) {
            renderer.parts.prewarm(format, samples);
        }
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

    /// The MSAA sample counts the scene can be drawn with here (its colour, the output's and depth's formats).
    pub fn msaa_sample_counts(&self) -> MsaaSampleCounts {
        self.msaa_counts
    }

    /// Changes the anti-aliasing: MLAA ([`PassId::Mlaa`] in the passes) or MSAA (the sample count), or none. Fails
    /// for an MSAA count the device lacks ([`AntiAliasing::supported`] picks one it has). A settings change: every
    /// pipeline it needs was made with the renderer, and the new attachments are made without waiting for the GPU.
    pub fn set_anti_aliasing(&mut self, aa: AntiAliasing) -> Result<(), RenderError> {
        let samples = aa.msaa_samples();
        self.check_msaa(samples)?;
        let passes = self.settings.passes.with_if(PassId::Mlaa, aa.mlaa());
        if (passes, samples) != (self.settings.passes, self.settings.msaa_samples) {
            self.settings.passes = passes;
            self.settings.msaa_samples = samples;
            self.rebuild();
        }
        Ok(())
    }

    /// The anti-aliasing in use: MSAA when the scene has more than one sample, MLAA when it runs, otherwise none.
    pub fn anti_aliasing(&self) -> AntiAliasing {
        match (self.settings.msaa_samples, self.plan.contains(PassId::Mlaa)) {
            (samples, _) if samples > 1 => AntiAliasing::Msaa(samples),
            (_, true) => AntiAliasing::Mlaa,
            _ => AntiAliasing::Off,
        }
    }

    /// Changes how the post chain finishes the image ([`crate::post`]): exposure and grade take effect with the next
    /// frame; a new bloom size or filter makes bloom's textures again (a settings change: nothing waits for the GPU,
    /// and every pipeline exists).
    pub fn set_post(&mut self, post: PostSettings) {
        if post == self.settings.post {
            return;
        }
        let old = self.settings.post;
        self.settings.post = post;
        if old.chain != post.chain {
            self.rebuild();
        } else if old.bloom != post.bloom || old.mlaa != post.mlaa {
            self.rebuild_post();
        }
    }

    /// What the post chain draws with: the scene colour's format, the anti-aliasing, bloom's levels, and how many
    /// passes and full-screen passes a frame has.
    pub fn post_stats(&self) -> PostStats {
        let bloom_passes = self.post.bloom.passes();
        let planned = self.plan.passes().len();
        let mut order = [None; PassId::COUNT];
        for (slot, pass) in order.iter_mut().zip(self.plan.passes()) {
            *slot = Some(pass.id);
        }
        PostStats {
            planned: order,
            scene_format: self.scene_target_format(),
            finish: self.plan.post(),
            msaa_samples: self.plan.msaa_samples(),
            mlaa: self.plan.contains(PassId::Mlaa),
            bloom_levels: self.post.bloom.levels() as u32,
            render_passes: (planned + bloom_passes.saturating_sub(1)) as u32,
            full_screen_passes: self.plan.full_screen_passes() as u32,
        }
    }

    /// Changes the colour the frame starts from (linear).
    pub fn set_clear_color(&mut self, color: wgpu::Color) {
        self.settings.clear_color = color;
    }

    /// Changes what the main view culls. Takes effect with the next frame.
    pub fn set_cull(&mut self, cull: CullSettings) {
        self.settings.cull = cull;
    }

    /// Changes how the round parts' levels of detail are chosen ([`crate::lod`]). Takes effect with the next frame;
    /// a new bias draws the sun's shadow maps again, whose casters' levels follow the camera's.
    pub fn set_lod(&mut self, lod: LodSettings) {
        self.settings.lod = lod;
    }

    /// Changes the scene's resolution: a render scale, or dynamic resolution ([`crate::resolution`]). Makes the
    /// scene's attachments again when their size changes (a settings change: it waits for nothing, and the textures
    /// in use are freed once the GPU is done with them); a new scale within dynamic resolution's range makes none.
    pub fn set_resolution(&mut self, resolution: ResolutionSettings) {
        if resolution == self.settings.resolution {
            return;
        }
        let old = self.settings.resolution;
        self.settings.resolution = resolution;
        self.resolution.set_settings(resolution);
        let (width, height) = (self.settings.width, self.settings.height);
        if old.scaled() != resolution.scaled()
            || self.clamped_texture_size(resolution.texture_size(width, height)) != self.texture_size
        {
            self.rebuild();
        }
    }

    /// The scene's resolution this frame: the render scale, the size drawn, and dynamic resolution's state.
    pub fn resolution_stats(&self) -> ResolutionStats {
        let settings = &self.settings.resolution;
        let (min_scale, max_scale) = settings.range();
        ResolutionStats {
            scale: if self.plan.scaled() {
                self.resolution.scale()
            } else {
                1.0
            },
            width: self.render_size.0,
            height: self.render_size.1,
            dynamic: settings.dynamic,
            target_ms: settings.target_ms,
            min_scale,
            max_scale,
            gpu_ms: self.resolution.last_gpu_ms(),
        }
    }

    /// Colours every part by the level of detail it is drawn at, a debug view: the Ball and the Cylinder from green
    /// (full detail) through yellow and orange to red (the coarsest level), every other kind grey. Off by default.
    pub fn set_lod_tint(&mut self, on: bool) {
        self.parts.set_lod_tint(on);
    }

    /// Whether parts are coloured by level of detail ([`Renderer::set_lod_tint`]).
    pub fn lod_tint(&self) -> bool {
        self.parts.lod_tint()
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

    /// Makes every [`Renderer::render_scene`] advance the local lights' fades ([`LightSettings::fade_time`]) by
    /// `seconds`, instead of by the wall time since the previous frame (`None`, the default). For tests and for
    /// recording frame by frame, where the wall time between frames says nothing about the game's time.
    pub fn set_light_fade_step(&mut self, seconds: Option<f32>) {
        self.parts.set_light_fade_step(seconds);
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
        let size = (self.settings.width, self.settings.height);
        let view = camera.filter(|_| cull.enabled).map(|camera| {
            CullView::for_camera(camera, scene.scene().origin(), size.0, size.1, cull).with_lod(&self.settings.lod)
        });
        // Shadows follow the levels the camera draws (none without a cull: every part at full detail).
        let lod = view.map_or(LodView::OFF, |view| view.lod);
        self.last_upload = scene.upload_view(&mut encoder, view.as_ref());
        self.last_cull = scene.cull_stats();
        // After the upload (the casters are culled from what its coalesce left), before the sun's shadows take the
        // scene's caster changes. Culling, levels of detail and the lights' ranking see the target's size, so a
        // render scale changes none of them; only the light clusters' pixels follow the size drawn.
        self.parts
            .prepare_lights(scene, camera, size, self.render_size, &self.settings.cull, &lod);
        self.parts.prepare_shadows(&mut encoder, scene, camera, size, &lod);
        self.shadow_in_use = u8::try_from(self.parts.shadow_stats().cascades).unwrap_or(0);
        // After the upload, which may have replaced the buffers the bind groups use.
        self.parts.prepare(
            scene,
            camera,
            self.settings.width,
            self.settings.height,
            // The scene's shaders write linear light into an HDR or sRGB target, and encode only for a plain one.
            self.scene_target_format().is_srgb() || crate::post::is_hdr(self.scene_target_format()),
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

    /// Records the shadow passes of this frame, before the frame's passes that sample the maps: a depth pass for
    /// every cascade whose casters are drawn (into its map, or a cached cascade's static map), and one for every
    /// cached cascade whose map is built (from its static map, just drawn or not, and its moving casters).
    fn record_shadow_passes(&mut self, encoder: &mut wgpu::CommandEncoder, slot: Option<usize>) {
        for cascade in 0..MAX_CASCADES {
            self.record_shadow_pass(encoder, slot, cascade, false);
            self.record_shadow_pass(encoder, slot, cascade, true);
        }
    }

    /// Records cascade `cascade`'s shadow pass when there is one this frame, cleared, then its casters drawn, or
    /// with `build` its map built.
    fn record_shadow_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        slot: Option<usize>,
        cascade: usize,
        build: bool,
    ) {
        let (view, label, pass) = if build {
            let view = self.parts.shadow_build_target(cascade);
            (view, SHADOW_BUILD_NAMES[cascade], Timed::Built(cascade as u8))
        } else {
            let view = self.parts.shadow_target(cascade);
            (view, SHADOW_PASS_NAMES[cascade], Timed::Cascade(cascade as u8))
        };
        let Some(view) = view else {
            return;
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
        if build {
            self.parts.build_shadow(cascade, &mut render_pass);
        } else {
            self.parts.draw_shadow(cascade, &mut render_pass);
        }
        drop(render_pass);
        if let Some(recorded) = self.shadow_passes.get_mut(self.shadow_recorded) {
            *recorded = pass;
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
        self.update_resolution();
        let encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic frame"),
        });
        (encoder, slot)
    }

    /// Chooses the size this frame draws the scene at: with dynamic resolution, from the newest GPU timings (just
    /// collected by the timer's `begin_frame`).
    fn update_resolution(&mut self) {
        if !self.plan.scaled() {
            self.render_size = (self.settings.width, self.settings.height);
            self.prepare_post();
            return;
        }
        if self.settings.resolution.dynamic
            && let Some((frame, gpu_ms)) = self
                .timer
                .as_ref()
                .and_then(GpuTimer::latest)
                .map(|timings| (timings.frame(), timings.total_ms()))
        {
            self.resolution.observe(frame, gpu_ms);
        }
        self.resolution.drawing(self.frame);
        let (width, height) = scaled_size(self.settings.width, self.settings.height, self.resolution.scale());
        self.render_size = (width.min(self.texture_size.0), height.min(self.texture_size.1));
        if self.plan.post() {
            self.prepare_post();
        } else {
            self.upscaler.prepare(&self.queue, self.render_size, self.texture_size);
        }
    }

    /// Writes the post chain's parameters for this frame's render size and settings, where they changed (usually
    /// nothing is written).
    fn prepare_post(&mut self) {
        if !self.plan.post() {
            return;
        }
        let post = &self.settings.post;
        let (render, texture) = (self.render_size, self.texture_size);
        let exposure = if post.exposure.is_finite() {
            post.exposure.max(0.0)
        } else {
            1.0
        };
        let bloom_levels = self.post.bloom.levels();
        let bloom_share = if bloom_levels > 0 {
            post.bloom.intensity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let format = self.settings.color_format;
        // Half an output level of dither where the output has 8-bit channels.
        let dither = match format.remove_srgb_suffix() {
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm => 1.0 / 255.0,
            wgpu::TextureFormat::Rgb10a2Unorm => 1.0 / 1023.0,
            _ => 0.0,
        };
        let [r, g, b] = post.tint;
        let params = FinishParams {
            rect: upscale_params(render, texture),
            params: [exposure, bloom_share, f32::from(u8::from(!format.is_srgb())), dither],
            tint: [r, g, b, 1.0 / bloom_levels.max(1) as f32],
            grade: [post.saturation, post.contrast, 0.0, 0.0],
            texel: [
                1.0 / texture.0.max(1) as f32,
                1.0 / texture.1.max(1) as f32,
                render.0 as f32,
                render.1 as f32,
            ],
        };
        self.post.finish.prepare(&self.queue, params);
        self.post.bloom.prepare(&self.queue, render, texture, exposure);
        self.post.mlaa.prepare(&self.queue, render, &post.mlaa, exposure);
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
        let scene_view = self.scene_color.as_ref().map(|attachment| &attachment.view);
        // Where the scene ends up: the target, or the scene colour the finish (or upscale) pass reads.
        let scene_output = if self.plan.uses_scene_color() {
            scene_view
        } else {
            Some(target)
        };
        // A frame drawn below the attachments' size draws into their top-left corner.
        let corner = (self.plan.scaled() && self.render_size != self.texture_size).then_some(self.render_size);

        for (index, pass) in self.plan.passes().iter().enumerate() {
            let timestamp_writes = match (self.timer.as_ref(), slot) {
                // The shadow passes recorded before took the first queries.
                (Some(timer), Some(slot)) => {
                    Some(timer.pass_writes(slot, self.shadow_recorded + usize::from(self.atlas_recorded) + index))
                }
                _ => None,
            };
            // The effects begin passes of their own, into their own textures.
            match pass.id {
                PassId::Mlaa => {
                    self.post.mlaa.record(encoder, timestamp_writes, corner);
                    continue;
                }
                PassId::Bloom => {
                    self.post.bloom.record(encoder, timestamp_writes);
                    continue;
                }
                _ => {}
            }
            let color = pass.color.and_then(|color| match color {
                ColorAttachment::Output(ops) => Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: color_ops(ops, clear_color),
                }),
                ColorAttachment::Scene(ops) => scene_view.map(|view| wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: color_ops(ops, clear_color),
                }),
                ColorAttachment::Msaa { ops, resolve } => msaa_view.map(|view| wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: if resolve { scene_output } else { None },
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
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(pass.id.name()),
                color_attachments: &colors[..has_color],
                depth_stencil_attachment: depth,
                timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some((width, height)) = corner
                && pass.id.is_scene()
            {
                render_pass.set_viewport(0.0, 0.0, width as f32, height as f32, 0.0, 1.0);
                render_pass.set_scissor_rect(0, 0, width, height);
            }
            if let Some(scene) = scene {
                // A merged scene pass draws each of the passes it stands for, in order.
                for id in pass.runs.iter() {
                    self.parts.draw(id, prepass, scene, &mut render_pass);
                }
            }
            match pass.id {
                PassId::Post => self.post.finish.draw(&mut render_pass),
                PassId::Upscale => self.upscaler.draw(&mut render_pass),
                _ => {}
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
            let cascades = self.shadow_passes[..self.shadow_recorded].iter().copied();
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
        // The upscale pass is the renderer's to plan, from the resolution settings.
        let resolution = self.settings.resolution;
        let passes = self.settings.passes.without(PassId::Upscale);
        let passes = if resolution.scaled() {
            passes.with(PassId::Upscale)
        } else {
            passes
        };
        self.plan = FramePlan::with_chain(passes, self.settings.post.chain, self.settings.msaa_samples);
        let (width, height) = (self.settings.width, self.settings.height);
        self.texture_size = if self.plan.scaled() {
            self.clamped_texture_size(resolution.texture_size(width, height))
        } else {
            (width, height)
        };
        self.render_size = self.texture_size;
        let scene_format = self.scene_target_format();
        self.parts.set_target(scene_format, self.plan.msaa_samples());
        if let Some(overlay) = self.overlay_pass.as_mut() {
            overlay.set_format(&self.device, self.settings.color_format);
        }
        // An attachment living only inside one pass (cleared, then discarded) is in tile memory only on tile-based
        // GPUs.
        let transient = |within_one_pass: bool| {
            if self.transient_attachments && within_one_pass {
                wgpu::TextureUsages::TRANSIENT_ATTACHMENT
            } else {
                wgpu::TextureUsages::empty()
            }
        };
        let samples = self.plan.msaa_samples();
        let depth_usage = transient(self.plan.depth_transient());
        let msaa_usage = transient(self.plan.msaa_color_transient());
        self.depth = self
            .plan
            .uses_depth()
            .then(|| self.attachment("metallic depth", DEPTH_FORMAT, samples, depth_usage));
        self.msaa_color = self
            .plan
            .uses_msaa_color()
            .then(|| self.attachment("metallic msaa colour", scene_format, samples, msaa_usage));
        // The finish pass (or the upscale pass) samples it.
        self.scene_color = self.plan.uses_scene_color().then(|| {
            self.attachment(
                "metallic scene colour",
                scene_format,
                1,
                wgpu::TextureUsages::TEXTURE_BINDING,
            )
        });
        let scene_view = self.scene_color.as_ref().map(|attachment| &attachment.view);
        self.upscaler
            .set_source(&self.device, scene_view.filter(|_| !self.plan.post()));
        self.rebuild_post();
    }

    /// Makes the post effects' textures for the plan and binds the finish pass's sources again. Part of a rebuild,
    /// or of a change to bloom's size or filter.
    fn rebuild_post(&mut self) {
        let scene_view = self
            .scene_color
            .as_ref()
            .map(|attachment| &attachment.view)
            .filter(|_| self.plan.post());
        let bloom = self.plan.contains(PassId::Bloom).then_some(&self.settings.post.bloom);
        self.post.rebuild(
            &self.device,
            &self.queue,
            scene_view,
            (self.texture_size, (self.settings.width, self.settings.height)),
            bloom,
            self.plan.contains(PassId::Mlaa),
        );
        // The new textures' parameters are written with the next frame.
        self.prepare_post();
    }

    /// The format the scene passes draw in: the HDR scene colour's with the finish pass, otherwise the output's.
    fn scene_target_format(&self) -> wgpu::TextureFormat {
        if self.plan.post() {
            self.hdr_format
        } else {
            self.settings.color_format
        }
    }

    /// `size` within the device's largest texture.
    fn clamped_texture_size(&self, (width, height): (u32, u32)) -> (u32, u32) {
        (width.min(self.max_texture_size), height.min(self.max_texture_size))
    }

    /// A scene attachment of the scene's attachment size with `samples` per pixel, usable as a render attachment and
    /// as `usage` says.
    fn attachment(
        &self,
        label: &str,
        format: wgpu::TextureFormat,
        samples: u32,
        usage: wgpu::TextureUsages,
    ) -> Attachment {
        let usage = usage | wgpu::TextureUsages::RENDER_ATTACHMENT;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: self.texture_size.0,
                height: self.texture_size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
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
/// The passes building cached cascades' maps, likewise.
const SHADOW_BUILD_NAMES: [&str; MAX_CASCADES] =
    ["shadow_0_build", "shadow_1_build", "shadow_2_build", "shadow_3_build"];

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
