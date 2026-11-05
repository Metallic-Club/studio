//! The frame loop: runs the planned passes into a target and keeps the GPU timings coming.

use crate::context::{Submitter, in_heap_turn};
use crate::cull::{CullSettings, CullStats, CullView};
use crate::depth::{DEPTH_CLEAR, DEPTH_FORMAT};
use crate::graph::{AttachmentOps, ColorAttachment, DepthOps, FramePlan, PassId, PassSet, PlannedPass};
use crate::lighting::SceneLighting;
use crate::lights::{LightSettings, LightStats};
use crate::lod::{LodSettings, LodView};
use crate::overlay::{Overlay, OverlayPass, overlay_recipe};
use crate::parts::{Camera, DrawStats, PartRenderer};
use crate::pipeline_cache::PipelineCacheStore;
use crate::pipelines::{Compiler, PartPipeline, Pipeline, PipelineKey, PipelineStats, Recipe};
use crate::post::{AntiAliasing, FinishParams, Post, PostSettings, PostStats};
use crate::resolution::{DynamicResolution, ResolutionSettings, ResolutionStats, scaled_size};
use crate::scene::{InstanceBinding, UploadStats};
use crate::shadow::{MAX_CASCADES, ShadowSettings, ShadowStats};
use crate::target::check_size;
use crate::textures::{Slot, TextureMaker, TextureSpec, TextureStats};
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

/// The scene's attachments for a set of settings: the plan they follow, their size, and the depth, MSAA colour and
/// scene colour textures it uses.
struct Attachments {
    plan: FramePlan,
    texture_size: (u32, u32),
    specs: [Option<TextureSpec>; 3],
}

/// The attachments of a settings change, while they are made off the frame ([`crate::textures`]): until all of them
/// exist, frames draw with the settings before it.
struct PendingAttachments {
    switch: Switch,
    attachments: Attachments,
    slots: [Option<Slot>; 3],
}

/// The settings that choose pipelines: a change to any of them takes effect once the pipelines it needs are compiled.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Switch {
    passes: PassSet,
    msaa_samples: u32,
    post: PostSettings,
    resolution: ResolutionSettings,
}

impl Switch {
    fn of(settings: &RendererSettings) -> Self {
        Self {
            passes: settings.passes,
            msaa_samples: settings.msaa_samples,
            post: settings.post,
            resolution: settings.resolution,
        }
    }

    /// The frame these settings plan: the upscale pass is the renderer's to plan, from the resolution settings.
    fn plan(&self) -> FramePlan {
        let passes = self.passes.without(PassId::Upscale);
        let passes = if self.resolution.scaled() {
            passes.with(PassId::Upscale)
        } else {
            passes
        };
        FramePlan::with_chain(passes, self.post.chain, self.msaa_samples)
    }
}

/// The most pipelines one frame's settings need: the scene's five without local lights, the finish pass, bloom's
/// three and MLAA's, or the upscale pass.
const MAX_NEEDED: usize = 10;

/// The most pipelines a scene's first frame waits for: its settings' and the lit ones (local lights, extra suns).
const FIRST_FRAME_KEYS: usize = MAX_NEEDED + PartPipeline::ALL_LIT.len();

/// Draws frames: the passes of a [`FramePlan`] into a target view, with per-pass GPU timing.
///
/// A frame allocates nothing on the heap in this crate and never waits for the GPU (ADR 0008 §9). Parts and the
/// sky are drawn by [`Renderer::render_scene`]; the post and UI systems record into the passes as they arrive.
///
/// # Pipelines
///
/// Every pipeline is compiled off the frame ([`crate::pipelines`]): [`Renderer::new`] returns once the ones its
/// settings draw with are compiled (on worker threads, all at once), and the rest (local lights, the overlay, the post
/// effects not in use, the scene at every other MSAA count, and extra suns once a scene has one) follow in the
/// background. A settings change whose pipelines are still compiling ([`Renderer::set_anti_aliasing`],
/// [`Renderer::set_passes`], [`Renderer::set_post`], [`Renderer::set_msaa_samples`], [`Renderer::set_resolution`])
/// waits for them while frames keep drawing with the settings before it: [`Renderer::settings`] and the stats say what
/// is drawn, and [`Renderer::pipeline_stats`] what is pending.
///
/// A scene's first frame never shows a setting or a light short: the first [`Renderer::render_scene`] after
/// [`Renderer::new`] (or [`Renderer::begin_scene`], for a newly loaded place) waits for the pipelines it draws with,
/// those of the settings asked for (a change still compiling included) and, when the scene has local lights or
/// `DirectionalLight`s, the lit ones, prioritized ahead of the rest. Later frames never wait: a light made while the
/// lit pipelines still compile (only possible in the first second or so) hurries them to the front of the queue, is
/// drawn once they exist, and (a local light) fades in over [`crate::LightSettings::fade_time`] rather than popping
/// on.
/// [`Renderer::wait_for_pipelines`] waits for everything, for tests, tools and loading screens.
///
/// # Textures
///
/// The large textures a settings change needs (the scene's attachments for a new MSAA count, scale or chain, the
/// sun's shadow maps, the lights' atlas) are made off the frame once a scene's first frame is drawn
/// (`textures`): one a frame, on a worker thread, while frames keep drawing with the settings before the
/// change; each part switches once all of its own exist. [`Renderer::texture_stats`] says what is still being made.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    submitter: Submitter,
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
    /// The shadow passes recorded this frame, in order (their queries come first): the cascades drawn and the
    /// cached cascades built.
    shadow_passes: [Timed; 2 * MAX_CASCADES],
    shadow_recorded: usize,
    /// Shadow cascades in use this frame.
    shadow_in_use: u8,
    /// The local lights' shadow atlas was drawn this frame (its query follows the cascades').
    atlas_recorded: bool,
    /// Compiles the pipelines off the frame.
    compiler: Compiler,
    /// The pipeline cache, where the device keeps one on disk.
    pipeline_cache: Option<std::sync::Arc<PipelineCacheStore>>,
    /// A settings change waiting for its pipelines; until they are compiled, frames draw with `settings`.
    waiting: Option<Switch>,
    /// The overlay's pipeline once compiled, kept while the overlay is off.
    overlay_pipeline: Option<wgpu::RenderPipeline>,
    /// The next [`Renderer::render_scene`] is a scene's first frame: it waits for the pipelines it draws with.
    first_scene_frame: bool,
    /// Makes the large textures of settings changes off the frame ([`crate::textures`]).
    maker: TextureMaker,
    /// Textures are made at once: until a scene's first frame is drawn.
    immediate: bool,
    /// The attachments of the waiting settings change, being made.
    pending_attachments: Option<PendingAttachments>,
    /// The scene format whose extra suns' pipelines are queued, at every MSAA count: none until a frame first draws
    /// a `DirectionalLight` ([`Renderer::prewarm_suns`]).
    suns_prewarmed: Option<wgpu::TextureFormat>,
}

impl Renderer {
    /// Creates a renderer for targets described by `settings`. Returns once the pipelines its settings draw with are
    /// compiled (on worker threads, at once), and compiles the rest in the background (see [`Renderer`]).
    pub fn new(context: &RenderContext, settings: RendererSettings) -> Result<Self, RenderError> {
        check_size(context, settings.width, settings.height)?;
        let hdr_format = crate::post::hdr_format(context);
        let msaa_counts = context
            .msaa_sample_counts(settings.color_format)
            .intersect(context.msaa_sample_counts(hdr_format))
            .intersect(context.msaa_sample_counts(DEPTH_FORMAT));
        if settings.msaa_samples != 1 && !msaa_counts.contains(settings.msaa_samples) {
            return Err(RenderError::UnsupportedTarget {
                format: settings.color_format,
                samples: settings.msaa_samples,
            });
        }
        let timer = if settings.gpu_timing {
            // A tiler's timestamps between draws mean nothing (`timing`): merged passes are timed as one there.
            let inside_passes = !context.capabilities().transient_attachments;
            GpuTimer::new(context.device(), context.queue(), inside_passes)
        } else {
            None
        };
        let pipeline_cache = context.pipeline_cache().cloned();
        // OpenGL compiles on its context's thread: there every compile runs here, before this returns.
        let gl = context.capabilities().backend == wgpu::Backend::Gl;
        let mut compiler = Compiler::new(
            context.device(),
            pipeline_cache.clone(),
            context.pipeline_turn().clone(),
            gl,
        );
        let first = Switch::of(&settings).plan();
        let first_format = if first.post() {
            hdr_format
        } else {
            settings.color_format
        };
        // OpenGL makes textures on its context's thread too: cheaply, inline (#152).
        let maker = TextureMaker::new(context.device(), !gl);
        // Queues the scene's first pipelines, then makes the shadows' and lights' while they compile.
        let parts = PartRenderer::new(
            (context.device(), context.queue(), &maker),
            InstanceBinding::for_context(context),
            (first_format, first.msaa_samples()),
            &mut compiler,
            pipeline_cache.as_deref().map(PipelineCacheStore::cache),
        );
        let mut renderer = Self {
            device: context.device().clone(),
            queue: context.queue().clone(),
            submitter: context.submitter(),
            settings,
            plan: FramePlan::new(settings.passes, 1),
            msaa_counts,
            max_texture_size: context.capabilities().max_texture_size,
            transient_attachments: context.capabilities().transient_attachments,
            depth: None,
            msaa_color: None,
            scene_color: None,
            hdr_format,
            post: Post::new(context, settings.color_format, hdr_format),
            texture_size: (settings.width, settings.height),
            render_size: (settings.width, settings.height),
            resolution: DynamicResolution::new(settings.resolution),
            upscaler: Upscaler::new(context.device(), settings.color_format),
            timer,
            parts,
            last_upload: UploadStats::default(),
            last_cull: CullStats::default(),
            frame: 0,
            overlay: Overlay::default(),
            overlay_pass: None,
            shadow_passes: [Timed::Cascade(0); 2 * MAX_CASCADES],
            shadow_recorded: 0,
            shadow_in_use: 0,
            atlas_recorded: false,
            compiler,
            pipeline_cache,
            waiting: None,
            overlay_pipeline: None,
            first_scene_frame: true,
            maker,
            immediate: true,
            pending_attachments: None,
            suns_prewarmed: None,
        };
        // The rest of what the first frame draws (the post chain's passes it runs), all at once on the workers; then
        // the lit pipelines of the same target behind them, which a scene with lights waits for in its first frame,
        // so they start now rather than after this returns. This returns when the first frame's alone are done.
        let switch = Switch::of(&settings);
        renderer.queue_needed(&switch);
        let (format, samples) = (first_format, first.msaa_samples());
        for pipeline in PartPipeline::LIT {
            renderer.queue(PipelineKey::Part {
                format,
                samples,
                pipeline,
            });
        }
        let (keys, count) = renderer.needed_keys(&switch, false, false);
        renderer.wait_for_keys(&keys[..count]);
        renderer.compiler.first_frame_ready();
        renderer.rebuild();
        renderer.parts.set_shadows(settings.shadows);
        renderer.parts.set_lights(settings.lights);
        renderer.prewarm();
        renderer.compiler.flush_inline();
        Ok(renderer)
    }

    /// Queues, in the background, every pipeline the renderer can switch to without a new colour format: first
    /// what a frame may need at any moment (local lights, the overlay, every post effect and quality, the upscale
    /// pass), then the scene at every other MSAA count the anti-aliasing settings offer (ADR 0008 §9). The extra
    /// suns' variants wait for a scene that has them ([`Renderer::prewarm_suns`]).
    fn prewarm(&mut self) {
        let (format, samples) = (self.scene_target_format(), self.plan.msaa_samples());
        for pipeline in PartPipeline::LIT {
            self.queue(PipelineKey::Part {
                format,
                samples,
                pipeline,
            });
        }
        if self.compiler.is_inline() {
            // Inline compiles would stall the frame that first draws an extra sun: here everything is compiled up front.
            self.prewarm_suns();
        }
        self.queue(PipelineKey::Overlay);
        for key in Post::KEYS {
            if self.post.prewarms(key) {
                self.queue(key);
            }
        }
        self.queue(PipelineKey::Upscale);
        for other in self.msaa_counts.iter().filter(|&n| n <= 8 && n != samples) {
            for pipeline in PartPipeline::BASE.into_iter().chain(PartPipeline::LIT) {
                let key = PipelineKey::Part {
                    format,
                    samples: other,
                    pipeline,
                };
                // After the same pipeline at the first count, whose shader stages it reuses.
                let after = PipelineKey::Part {
                    format,
                    samples,
                    pipeline,
                };
                self.queue_after(key, Some(after));
            }
        }
    }

    /// Queues, in the background, the extra suns' variants of the scene's pipelines at the format drawn into: first at
    /// the MSAA count in use, then at every other one. Once per format, when a frame first wants extra suns: a game
    /// without a `DirectionalLight` never compiles them (on DX12 they would triple the background compiles' CPU time
    /// after the first frame, `docs/perf.md`). Where compiles run inline (OpenGL) they are compiled up front with the
    /// rest instead ([`Renderer::prewarm`]), so no frame stalls on them.
    fn prewarm_suns(&mut self) {
        let (format, samples) = (self.scene_target_format(), self.plan.msaa_samples());
        if self.suns_prewarmed == Some(format) {
            return;
        }
        self.suns_prewarmed = Some(format);
        for pipeline in PartPipeline::SUNS.into_iter().chain(PartPipeline::LIT_SUNS) {
            self.queue(PipelineKey::Part {
                format,
                samples,
                pipeline,
            });
        }
        for other in self.msaa_counts.iter().filter(|&n| n <= 8 && n != samples) {
            for pipeline in PartPipeline::SUNS.into_iter().chain(PartPipeline::LIT_SUNS) {
                let key = PipelineKey::Part {
                    format,
                    samples: other,
                    pipeline,
                };
                let after = PipelineKey::Part {
                    format,
                    samples,
                    pipeline,
                };
                self.queue_after(key, Some(after));
            }
        }
    }

    /// Whether the pipeline `key` is compiled.
    fn has(&self, key: PipelineKey) -> bool {
        match key {
            PipelineKey::Part {
                format,
                samples,
                pipeline,
            } => self.parts.has(format, samples, pipeline),
            PipelineKey::Upscale => self.upscaler.has(),
            PipelineKey::Overlay => self.overlay_pipeline.is_some(),
            _ => self.post.has(key),
        }
    }

    /// The compile of the pipeline `key`.
    fn recipe(&mut self, key: PipelineKey) -> Option<Recipe> {
        match key {
            PipelineKey::Part {
                format,
                samples,
                pipeline,
            } => Some(self.parts.recipe(format, samples, pipeline)),
            PipelineKey::Upscale => Some(self.upscaler.full_screen().recipe()),
            PipelineKey::Overlay => Some(overlay_recipe(self.settings.color_format)),
            _ => self.post.recipe(key),
        }
    }

    /// Queues the compile of `key` unless it is compiled or queued; a scene pipeline after the one of its target whose
    /// shader stages it reuses ([`PartPipeline::shares_stages_of`]).
    fn queue(&mut self, key: PipelineKey) {
        let after = match key {
            PipelineKey::Part {
                format,
                samples,
                pipeline,
            } => pipeline.shares_stages_of().map(|pipeline| PipelineKey::Part {
                format,
                samples,
                pipeline,
            }),
            _ => None,
        };
        self.queue_after(key, after);
    }

    /// Queues the compile of `key` unless it is compiled or queued, to start once `after` is compiled.
    fn queue_after(&mut self, key: PipelineKey, after: Option<PipelineKey>) {
        if !self.has(key)
            && !self.compiler.is_queued(key)
            && let Some(recipe) = self.recipe(key)
        {
            self.compiler.submit(key, recipe, after);
        }
    }

    /// Calls `f` with every pipeline a frame drawn with `switch` needs, but the local lights' (frames draw without
    /// them until they are compiled).
    fn needs(&self, switch: &Switch, mut f: impl FnMut(PipelineKey)) {
        let plan = switch.plan();
        let format = if plan.post() {
            self.hdr_format
        } else {
            self.settings.color_format
        };
        let samples = plan.msaa_samples();
        for pipeline in PartPipeline::BASE {
            f(PipelineKey::Part {
                format,
                samples,
                pipeline,
            });
        }
        if plan.post() {
            let (bloom, mlaa) = (plan.contains(PassId::Bloom), plan.contains(PassId::Mlaa));
            f(PipelineKey::Finish(u8::from(bloom) | u8::from(mlaa) << 1));
            if bloom {
                let size = (self.settings.width, self.settings.height);
                self.post
                    .bloom
                    .keys(&switch.post.bloom, size)
                    .into_iter()
                    .for_each(&mut f);
            }
            if mlaa {
                f(PipelineKey::Mlaa);
            }
        } else if plan.contains(PassId::Upscale) {
            f(PipelineKey::Upscale);
        }
    }

    /// Whether every pipeline a frame drawn with `switch` needs is compiled.
    fn ready_for(&self, switch: &Switch) -> bool {
        let mut ready = true;
        self.needs(switch, |key| ready &= self.has(key));
        ready
    }

    /// Queues every pipeline a frame drawn with `switch` needs and lacks.
    fn queue_needed(&mut self, switch: &Switch) {
        let mut keys = [None; MAX_NEEDED];
        let mut count = 0;
        self.needs(switch, |key| {
            if let Some(slot) = keys.get_mut(count) {
                *slot = Some(key);
                count += 1;
            }
        });
        for key in keys.into_iter().flatten() {
            self.queue(key);
        }
    }

    /// Takes the compiles that finished and puts their pipelines in place, without waiting; then makes a settings
    /// change that was waiting for them. Allocates nothing (a pipeline's slot exists from when it was queued).
    fn take_compiled(&mut self) {
        let Self {
            compiler,
            parts,
            post,
            upscaler,
            overlay_pass,
            overlay_pipeline,
            ..
        } = self;
        let mut installed = false;
        compiler.drain(|key, pipeline| {
            install(parts, post, upscaler, (overlay_pass, overlay_pipeline), key, pipeline);
            installed = true;
        });
        if installed {
            self.apply_waiting();
        }
    }

    /// Waits until no compile is queued or running, putting each pipeline in place as it finishes; then makes a
    /// settings change that was waiting.
    fn wait_for_compiles(&mut self) {
        let Self {
            compiler,
            parts,
            post,
            upscaler,
            overlay_pass,
            overlay_pipeline,
            ..
        } = self;
        compiler.wait(|key, pipeline| {
            install(parts, post, upscaler, (overlay_pass, overlay_pipeline), key, pipeline);
        });
        self.apply_waiting();
    }

    /// Makes the settings change that was waiting, once its pipelines are compiled and its attachments made.
    fn apply_waiting(&mut self) {
        if self.waiting.is_some() {
            self.advance_switch();
        }
    }

    /// Moves the waiting settings change on: once its pipelines are compiled, asks for its attachments (made off the
    /// frame when large, [`crate::textures`]), and once they exist makes it the one drawn.
    fn advance_switch(&mut self) {
        let Some(next) = self.waiting else {
            return;
        };
        if !self.ready_for(&next) {
            return;
        }
        let attachments = if self.needs_rebuild(&next) {
            if self.pending_attachments.as_ref().is_none_or(|p| p.switch != next) {
                self.cancel_attachments();
                let attachments = self.attachments(&next);
                let slots = attachments
                    .specs
                    .map(|spec| spec.map(|spec| Slot::new(self.maker.make(spec))));
                self.pending_attachments = Some(PendingAttachments {
                    switch: next,
                    attachments,
                    slots,
                });
            }
            let Some(pending) = self.pending_attachments.as_mut() else {
                return;
            };
            let mut here = true;
            for slot in pending.slots.iter_mut().flatten() {
                here &= slot.poll(&self.maker);
            }
            if !here {
                return;
            }
            self.pending_attachments.take().map(|pending| {
                (
                    pending.attachments,
                    pending.slots.map(|slot| slot.and_then(Slot::into_texture)),
                )
            })
        } else {
            // Those of a change it replaced are not needed.
            self.cancel_attachments();
            None
        };
        self.waiting = None;
        self.apply(next, attachments);
    }

    /// Gives up the attachments being made for a settings change (a newer one, or a new target size, replaced it).
    fn cancel_attachments(&mut self) {
        if let Some(pending) = self.pending_attachments.take() {
            for slot in pending.slots.into_iter().flatten() {
                slot.cancel(&self.maker);
            }
        }
    }

    /// Makes the next [`Renderer::render_scene`] a scene's first frame, as the first one after [`Renderer::new`] is:
    /// it waits for exactly the pipelines it draws with (those of the settings asked for, a change still compiling
    /// included, and the local lights' and extra suns' when the scene has any), so the first frame of a game or a
    /// newly loaded place already looks as it should. Textures are made at once again until it is drawn. Call it when a new scene is
    /// attached; later frames never wait.
    pub fn begin_scene(&mut self) {
        self.first_scene_frame = true;
        self.set_immediate(true);
    }

    /// Makes textures at once (until a scene's first frame is drawn) or off the frame.
    fn set_immediate(&mut self, immediate: bool) {
        if self.immediate != immediate {
            self.immediate = immediate;
            self.maker.set_immediate(immediate);
        }
    }

    /// The first frame of a scene: waits until the pipelines it draws `scene` with are compiled (prioritized, on the
    /// workers) and the textures still being made exist, and makes a settings change that waited for them.
    fn wait_for_scene(&mut self, scene: &GpuScene) {
        self.maker.finish();
        let lit = self.settings.lights.enabled && scene.scene().light_count() > 0;
        let suns = self.settings.lights.directional_budget() > 0
            && scene.scene().directional_lights().iter().any(|light| light.shines());
        let (keys, count) = self.needed_keys(&self.wanted(), lit, suns);
        self.wait_for_keys(&keys[..count]);
    }

    /// The pipelines a frame drawn with `switch` needs, with the lit ones when `lit` (local lights) and the extra
    /// suns' when `suns`: an array and how much of it holds them.
    fn needed_keys(&self, switch: &Switch, lit: bool, suns: bool) -> ([PipelineKey; FIRST_FRAME_KEYS], usize) {
        let mut keys = [PipelineKey::Overlay; FIRST_FRAME_KEYS];
        let mut count = 0;
        let mut push = |key| {
            if let Some(slot) = keys.get_mut(count) {
                *slot = key;
                count += 1;
            }
        };
        self.needs(switch, &mut push);
        let plan = switch.plan();
        let format = if plan.post() {
            self.hdr_format
        } else {
            self.settings.color_format
        };
        let lit_keys: &[PartPipeline] = if lit { &PartPipeline::LIT } else { &[] };
        let sun_keys: &[PartPipeline] = match (lit, suns) {
            (_, false) => &[],
            (false, true) => &PartPipeline::SUNS,
            (true, true) => &PartPipeline::LIT_SUNS,
        };
        for &pipeline in lit_keys.iter().chain(sun_keys) {
            push(PipelineKey::Part {
                format,
                samples: plan.msaa_samples(),
                pipeline,
            });
        }
        (keys, count)
    }

    /// Queues what `keys` lacks, waits for all of it (prioritized), and makes a settings change that waited for it.
    fn wait_for_keys(&mut self, keys: &[PipelineKey]) {
        for &key in keys {
            self.queue(key);
        }
        let Self {
            compiler,
            parts,
            post,
            upscaler,
            overlay_pass,
            overlay_pipeline,
            ..
        } = self;
        compiler.wait_for(keys, |key, pipeline| {
            install(parts, post, upscaler, (overlay_pass, overlay_pipeline), key, pipeline);
        });
        self.apply_waiting();
    }

    /// Moves the lit pipelines of the target drawn into to the front of the compile queue: a frame has local lights
    /// or extra suns to draw and draws without them until they exist (a light made while they still compile, #86).
    fn hurry_lights(&mut self) {
        let (format, samples) = (self.scene_target_format(), self.plan.msaa_samples());
        let keys = PartPipeline::ALL_LIT.map(|pipeline| PipelineKey::Part {
            format,
            samples,
            pipeline,
        });
        // The lit ones are queued from the start; the extra suns' may not be yet at this format.
        for &key in &keys {
            self.queue(key);
        }
        self.compiler.prioritize(&keys);
    }

    /// Waits for every pipeline compile queued (the background ones included) and every texture a settings change is
    /// making off the frame (`textures`), and makes a settings change that was waiting for them (the sun's
    /// shadows and the lights' atlas take theirs with the next frame). Blocks for as long as they take, on the CPU (it
    /// never waits for the GPU): for tests, tools and loading screens, never in a game's frame loop. Resumes paused
    /// compiles.
    pub fn wait_for_pipelines(&mut self) {
        self.maker.finish();
        self.wait_for_compiles();
    }

    /// Whether the local lights' pipelines of the target drawn into are compiled (a frame with lights draws with
    /// them). Another renderer on the same device that draws lights, such as Auto's calibration workload (#139),
    /// waits for them before compiling its own: they share their shaders, which a device such as DX12's compiles
    /// once and keeps, where compiling them at the same moment compiles them twice.
    pub fn lit_pipelines_ready(&self) -> bool {
        let (format, samples) = (self.scene_target_format(), self.plan.msaa_samples());
        PartPipeline::LIT.into_iter().all(|pipeline| {
            self.has(PipelineKey::Part {
                format,
                samples,
                pipeline,
            })
        })
    }

    /// What the textures made off the frame have done and what is still being made (`textures`).
    pub fn texture_stats(&self) -> TextureStats {
        self.maker.stats()
    }

    /// Stops starting background compiles (those already running finish) until called again with `false`: for a
    /// game that wants every core to itself for a while, and for tests of the frames drawn while compiles are
    /// pending.
    pub fn pause_pipeline_compiles(&mut self, paused: bool) {
        self.compiler.pause(paused);
    }

    /// What the pipeline compiles have done and what is pending, for the overlay and reports.
    pub fn pipeline_stats(&self) -> PipelineStats {
        PipelineStats {
            switch_waiting: self.waiting.is_some(),
            cache_loaded: self.pipeline_cache.as_ref().map(|cache| cache.loaded()),
            ..self.compiler.stats()
        }
    }

    /// The settings frames are drawn with. A change waiting for its pipelines ([`PipelineStats::switch_waiting`]) is
    /// not in them until it takes effect.
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

    /// The per-pass GPU timings of up to [`crate::TIMING_RING_SIZE`] of the newest frames collected, in no particular
    /// order: for a caller that draws several frames between two of its reads and wants each one's timings
    /// ([`Renderer::gpu_timings`] keeps only the newest), as Auto's calibration does. A frame's timings stay until a
    /// later frame's take their place.
    pub fn recent_gpu_timings(&self) -> impl Iterator<Item = &FrameTimings> {
        self.timer.iter().flat_map(GpuTimer::recent)
    }

    /// Changes the target size (on window resize). Recreates the internal attachments when the size changes.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), RenderError> {
        if width == 0 || height == 0 || width > self.max_texture_size || height > self.max_texture_size {
            return Err(RenderError::InvalidSize { width, height });
        }
        if (width, height) != (self.settings.width, self.settings.height) {
            self.settings.width = width;
            self.settings.height = height;
            // Attachments being made for a change are of the old size: asked for again at this one.
            self.cancel_attachments();
            self.rebuild();
        }
        Ok(())
    }

    /// Changes which passes run, once the pipelines they need are compiled (see [`Renderer`]).
    pub fn set_passes(&mut self, passes: PassSet) {
        let mut next = self.wanted();
        next.passes = passes;
        self.switch(next);
    }

    /// Changes the scene's MSAA sample count (1 turns MSAA off), once the pipelines it needs are compiled.
    pub fn set_msaa_samples(&mut self, samples: u32) -> Result<(), RenderError> {
        self.check_msaa(samples)?;
        let mut next = self.wanted();
        next.msaa_samples = samples;
        self.switch(next);
        Ok(())
    }

    /// The settings that choose pipelines, as last asked for: a change waiting for its pipelines, or those drawn.
    fn wanted(&self) -> Switch {
        self.waiting.unwrap_or_else(|| Switch::of(&self.settings))
    }

    /// Draws with `next` from now on when its pipelines are compiled and its attachments made, otherwise queues what
    /// it lacks and draws as before until it is ready (the first frame after that switches).
    fn switch(&mut self, next: Switch) {
        if next == Switch::of(&self.settings) {
            self.waiting = None;
            self.cancel_attachments();
            return;
        }
        if !self.ready_for(&next) {
            self.queue_needed(&next);
        }
        self.waiting = Some(next);
        self.advance_switch();
    }

    /// Whether drawing with `next` needs new attachments and a new frame plan.
    fn needs_rebuild(&self, next: &Switch) -> bool {
        let old = Switch::of(&self.settings);
        let (width, height) = (self.settings.width, self.settings.height);
        let rescaled = old.resolution != next.resolution
            && (old.resolution.scaled() != next.resolution.scaled()
                || self.clamped_texture_size(next.resolution.texture_size(width, height)) != self.texture_size);
        old.passes != next.passes
            || old.msaa_samples != next.msaa_samples
            || old.post.chain != next.post.chain
            || rescaled
    }

    /// Draws with `next` from now on (its pipelines are compiled): installs its `attachments` (made for it, or made
    /// now when `None` and it needs them) and the frame plan, or makes the post effects' textures again. A settings
    /// change: it waits for nothing.
    fn apply(&mut self, next: Switch, attachments: Option<(Attachments, [Option<wgpu::Texture>; 3])>) {
        let old = Switch::of(&self.settings);
        let rebuild = self.needs_rebuild(&next);
        self.settings.passes = next.passes;
        self.settings.msaa_samples = next.msaa_samples;
        self.settings.post = next.post;
        self.settings.resolution = next.resolution;
        if old.resolution != next.resolution {
            self.resolution.set_settings(next.resolution);
        }
        if rebuild {
            match attachments {
                Some((attachments, textures)) => self.rebuild_with(attachments, textures),
                None => self.rebuild(),
            }
        } else if old.post.bloom != next.post.bloom || old.post.mlaa != next.post.mlaa {
            self.rebuild_post();
        }
    }

    /// The MSAA sample counts the scene can be drawn with here (its colour, the output's and depth's formats).
    pub fn msaa_sample_counts(&self) -> MsaaSampleCounts {
        self.msaa_counts
    }

    /// Changes the anti-aliasing: MLAA ([`PassId::Mlaa`] in the passes) or MSAA (the sample count), or none. Fails
    /// for an MSAA count the device lacks ([`AntiAliasing::supported`] picks one it has). A settings change: its
    /// pipelines are compiled in the background with the renderer's, and its attachments made off the frame (until
    /// both are, frames keep the anti-aliasing before it); nothing waits for the GPU.
    pub fn set_anti_aliasing(&mut self, aa: AntiAliasing) -> Result<(), RenderError> {
        let samples = aa.msaa_samples();
        self.check_msaa(samples)?;
        let mut next = self.wanted();
        next.passes = next.passes.with_if(PassId::Mlaa, aa.mlaa());
        next.msaa_samples = samples;
        self.switch(next);
        Ok(())
    }

    /// The anti-aliasing drawn: MSAA when the scene has more than one sample, MLAA when it runs, otherwise none.
    pub fn anti_aliasing(&self) -> AntiAliasing {
        match (self.settings.msaa_samples, self.plan.contains(PassId::Mlaa)) {
            (samples, _) if samples > 1 => AntiAliasing::Msaa(samples),
            (_, true) => AntiAliasing::Mlaa,
            _ => AntiAliasing::Off,
        }
    }

    /// Changes how the post chain finishes the image ([`crate::post`]): exposure and grade take effect with the next
    /// frame; a new bloom size or filter makes bloom's textures again (a settings change: nothing waits for the GPU)
    /// once its pipelines are compiled.
    pub fn set_post(&mut self, post: PostSettings) {
        let mut next = self.wanted();
        next.post = post;
        self.switch(next);
    }

    /// What the post chain draws with: the scene colour's format, the anti-aliasing, bloom's levels, and how many
    /// passes and full-screen passes a frame has.
    pub fn post_stats(&self) -> PostStats {
        // Bloom's planned pass is its chain: render passes of its own, or one compute pass.
        let bloom_planned = usize::from(self.plan.contains(PassId::Bloom));
        let bloom_passes = self.post.bloom.render_passes();
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
            render_passes: (planned + bloom_passes).saturating_sub(bloom_planned) as u32,
            compute_passes: self.post.bloom.compute_passes() as u32,
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
    /// Without the post chain, a render scale needs the upscale pass's pipeline (compiled in the background).
    pub fn set_resolution(&mut self, resolution: ResolutionSettings) {
        let mut next = self.wanted();
        next.resolution = resolution;
        self.switch(next);
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
    /// size or cascade count makes the maps again (off the frame once a scene's first frame is drawn, see
    /// [`Renderer`]), and every map is drawn again once they exist.
    pub fn set_shadows(&mut self, shadows: ShadowSettings) {
        if shadows != self.settings.shadows {
            self.settings.shadows = shadows;
            self.parts.set_shadows(shadows);
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
    /// Sync the scene ([`GpuScene::sync`]) first. Never blocks on the GPU; see [`Renderer::render`] about pacing. A
    /// scene's first frame (the first after [`Renderer::new`] or [`Renderer::begin_scene`]) waits on the CPU for the
    /// pipelines it draws with, so it looks as every later frame does.
    pub fn render_scene(
        &mut self,
        target: &wgpu::TextureView,
        scene: &mut GpuScene,
        camera: Option<&Camera>,
    ) -> wgpu::SubmissionIndex {
        if std::mem::take(&mut self.first_scene_frame) {
            self.wait_for_scene(scene);
        }
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
        if self.parts.wants_suns() {
            self.prewarm_suns();
        }
        if self.parts.lights_held() {
            self.hurry_lights();
        }
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
    /// with `build` its map built (loaded rather than cleared when the build restores footprints, #133).
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
        // A build that restores footprints keeps the rest of what the map holds (#133).
        let load = if build && self.parts.shadow_build_restores(cascade) {
            wgpu::LoadOp::Load
        } else {
            wgpu::LoadOp::Clear(1.0)
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
                    load,
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
    /// pass set has [`PassId::Ui`] show it. Turning it on makes its buffer (a settings change, not something to do
    /// every frame); its pipeline is compiled in the background with the renderer's, and it is drawn once that is
    /// done.
    pub fn set_overlay_enabled(&mut self, enabled: bool) {
        if enabled && self.overlay_pass.is_none() {
            self.overlay.reserve();
            self.overlay_pass = Some(OverlayPass::new(&self.device, self.overlay_pipeline.clone()));
            self.queue(PipelineKey::Overlay);
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

    /// Starts a frame: the pipelines compiled since the last (and a settings change that waited for them), its GPU
    /// timing slot, if any, and its encoder.
    fn begin_frame(&mut self) -> (wgpu::CommandEncoder, Option<usize>) {
        if self.compiler.busy() {
            self.take_compiled();
        }
        self.shadow_recorded = 0;
        self.shadow_in_use = 0;
        self.atlas_recorded = false;
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("metallic frame"),
        });
        // A texture a settings change waits for, made since the last frame: cleared first, then taken by its system
        // (#152). Touches nothing when none is pending.
        self.maker.begin_frame(&mut encoder);
        if self.waiting.is_some() {
            self.advance_switch();
        }
        let slot = match self.timer.as_mut() {
            Some(timer) => timer.begin_frame(&self.device),
            None => None,
        };
        self.update_resolution();
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

        // The shadow passes recorded before took the first queries.
        let mut timed = self.shadow_recorded + usize::from(self.atlas_recorded);
        let inside = self.timer.as_ref().is_some_and(GpuTimer::inside_passes);
        for pass in self.plan.passes() {
            let count = timed_passes(pass, inside).count();
            let first = timed;
            timed += count;
            let timestamp_writes = match (self.timer.as_ref(), slot) {
                (Some(timer), Some(slot)) => Some(timer.span_writes(slot, first, timed - 1)),
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
            // A merged pass draws each of the passes it stands for, in order, timed apart where it can be.
            for (i, id) in pass.runs.iter().enumerate() {
                if i > 0
                    && inside
                    && let (Some(timer), Some(slot)) = (self.timer.as_ref(), slot)
                {
                    timer.write_between(&mut render_pass, slot, first + i - 1);
                }
                match id {
                    PassId::Post => self.post.finish.draw(&mut render_pass),
                    PassId::Upscale => self.upscaler.draw(&mut render_pass),
                    PassId::Ui => {
                        if let Some(overlay) = self.overlay_pass.as_ref() {
                            overlay.draw(&mut render_pass);
                        }
                    }
                    _ => {
                        if let Some(scene) = scene {
                            self.parts.draw(id, prepass, scene, &mut render_pass);
                        }
                    }
                }
            }
            drop(render_pass);
        }
    }

    /// Ends the frame: resolves its timestamps, submits it and counts it.
    fn end_frame(&mut self, mut encoder: wgpu::CommandEncoder, slot: Option<usize>) -> wgpu::SubmissionIndex {
        if let (Some(timer), Some(slot)) = (self.timer.as_mut(), slot) {
            let cascades = self.shadow_passes[..self.shadow_recorded].iter().copied();
            let atlas = self.atlas_recorded.then_some(Timed::LightShadows);
            let inside = timer.inside_passes();
            let passes = self
                .plan
                .passes()
                .iter()
                .flat_map(move |pass| timed_passes(pass, inside))
                .map(Timed::Pass);
            timer.end_frame(
                &mut encoder,
                slot,
                self.frame,
                cascades.chain(atlas).chain(passes),
                self.shadow_in_use,
            );
        }
        let submission = self.submitter.submit(std::iter::once(encoder.finish()));
        if let (Some(timer), Some(slot)) = (self.timer.as_ref(), slot) {
            timer.after_submit(slot);
        }
        // The next texture a settings change needs, made while the frame is presented (#152); from now on large
        // ones are made so.
        self.maker.kick();
        self.set_immediate(false);
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

    /// Plans the frame again and makes the attachments it needs, now. Runs on settings changes (made at once: a new
    /// target size, or a renderer's first settings), never per frame.
    fn rebuild(&mut self) {
        let attachments = self.attachments(&Switch::of(&self.settings));
        let textures = attachments
            .specs
            .map(|spec| spec.map(|spec| spec.make_now(&self.device)));
        self.rebuild_with(attachments, textures);
    }

    /// The frame plan of `switch` at the target's size, and the scene attachments it needs: the depth, the MSAA
    /// colour and the scene colour.
    fn attachments(&self, switch: &Switch) -> Attachments {
        let plan = switch.plan();
        let (width, height) = (self.settings.width, self.settings.height);
        let texture_size = if plan.scaled() {
            self.clamped_texture_size(switch.resolution.texture_size(width, height))
        } else {
            (width, height)
        };
        let scene_format = if plan.post() {
            self.hdr_format
        } else {
            self.settings.color_format
        };
        // An attachment living only inside one pass (cleared, then discarded) is in tile memory only on tile-based
        // GPUs.
        let transient = |within_one_pass: bool| {
            if self.transient_attachments && within_one_pass {
                wgpu::TextureUsages::TRANSIENT_ATTACHMENT
            } else {
                wgpu::TextureUsages::empty()
            }
        };
        let samples = plan.msaa_samples();
        let spec = |label, format, samples, usage| {
            TextureSpec::new(
                label,
                (texture_size.0, texture_size.1, 1),
                format,
                usage | wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
            .multisampled(samples)
        };
        let specs = [
            plan.uses_depth().then(|| {
                spec(
                    "metallic depth",
                    DEPTH_FORMAT,
                    samples,
                    transient(plan.depth_transient()),
                )
            }),
            plan.uses_msaa_color().then(|| {
                spec(
                    "metallic msaa colour",
                    scene_format,
                    samples,
                    transient(plan.msaa_color_transient()),
                )
            }),
            // The finish pass (or the upscale pass) samples it.
            plan.uses_scene_color().then(|| {
                spec(
                    "metallic scene colour",
                    scene_format,
                    1,
                    wgpu::TextureUsages::TEXTURE_BINDING,
                )
            }),
        ];
        Attachments {
            plan,
            texture_size,
            specs,
        }
    }

    /// Draws with `attachments`' plan from now on, into `textures` (made for its specs).
    fn rebuild_with(&mut self, attachments: Attachments, textures: [Option<wgpu::Texture>; 3]) {
        self.plan = attachments.plan;
        self.texture_size = attachments.texture_size;
        self.render_size = self.texture_size;
        self.parts
            .set_target(self.scene_target_format(), self.plan.msaa_samples());
        let [depth, msaa_color, scene_color] = textures.map(|texture| {
            texture.map(|texture| Attachment {
                view: in_heap_turn(|| texture.create_view(&wgpu::TextureViewDescriptor::default())),
            })
        });
        self.depth = depth;
        self.msaa_color = msaa_color;
        self.scene_color = scene_color;
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
}

/// Puts the compiled pipeline `key` where it is drawn from. Allocates nothing.
fn install(
    parts: &mut PartRenderer,
    post: &mut Post,
    upscaler: &mut Upscaler,
    (overlay_pass, overlay_pipeline): (&mut Option<OverlayPass>, &mut Option<wgpu::RenderPipeline>),
    key: PipelineKey,
    pipeline: Pipeline,
) {
    match key {
        PipelineKey::Part {
            format,
            samples,
            pipeline: which,
        } => {
            if let Some(pipeline) = pipeline.render() {
                parts.install(format, samples, which, pipeline);
            }
        }
        PipelineKey::Upscale => {
            if let Some(pipeline) = pipeline.render() {
                upscaler.install(pipeline);
            }
        }
        PipelineKey::Overlay => {
            if let Some(pipeline) = pipeline.render() {
                if let Some(pass) = overlay_pass.as_mut() {
                    pass.set_pipeline(pipeline.clone());
                }
                *overlay_pipeline = Some(pipeline);
            }
        }
        _ => post.install(key, pipeline),
    }
}

/// The shadow passes' names in GPU debuggers.
const SHADOW_PASS_NAMES: [&str; MAX_CASCADES] = ["shadow_0", "shadow_1", "shadow_2", "shadow_3"];
/// The passes building cached cascades' maps, likewise.
const SHADOW_BUILD_NAMES: [&str; MAX_CASCADES] =
    ["shadow_0_build", "shadow_1_build", "shadow_2_build", "shadow_3_build"];

/// The passes `pass` reports GPU time for, in order: each it runs when timestamps are written `inside` passes,
/// otherwise the render pass as a whole.
fn timed_passes(pass: &PlannedPass, inside: bool) -> impl Iterator<Item = PassId> {
    if inside {
        pass.runs
    } else {
        PassSet::EMPTY.with(pass.id)
    }
    .iter()
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
