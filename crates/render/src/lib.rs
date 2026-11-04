//! The renderer: a mirror of the data model, drawn with `wgpu`.
//!
//! Owns: the GPU device and surfaces (from a raw window handle), meshes, materials, lights, shadows, culling,
//! batching, graphics quality presets and dynamic resolution. Implements the render side of components
//! (`SpotLightEmitter`, ...).
//!
//! Must not depend on: the platform, project, scripting or physics crates. The engine hands it a window handle and
//! decoded assets. See `docs/adr/0004-runtime-rendering-platform.md`; how it stays fast is fixed by
//! `docs/adr/0008-rendering-performance.md`.
//!
//! # Pieces
//!
//! - [`RenderContext`]: the instance, adapter, device and queue, created under a process-wide lock, with the
//!   adapter's [`Capabilities`]. It renders offscreen ([`RenderContext::new_offscreen`]) or to a window
//!   ([`RenderContext::new_with_window`], which also returns the window's [`RenderSurface`]).
//! - [`Renderer`]: runs one frame as a fixed list of passes ([`PassId`]) with explicit attachments; load and store
//!   operations come from [`FramePlan`], which discards whatever no later pass reads.
//! - Reverse-Z depth ([`depth`]): `Depth32Float`, cleared to 0, tested with `Greater`.
//! - GPU timing: timestamps around every pass, read back asynchronously a frame or more later
//!   ([`FrameTimings`]). Nothing in a frame waits for the GPU.
//! - The GPU-resident scene ([`scene`]): every renderable instance keeps a slot in an instance buffer, and only
//!   what the data model's change tracking names is repacked and uploaded ([`GpuScene`], [`InstanceScene`]).
//! - Drawing parts ([`parts`], [`Renderer::render_scene`]): one instanced draw per mesh kind and level of detail over
//!   the scene's draw lists, a material table, PBR-lite shading, a sorted transparent pass, and the camera from
//!   `Workspace.CurrentCamera` ([`CameraReader`]).
//! - Culling ([`cull`], [`RendererSettings::cull`]): a loose grid of every drawn instance's bounding sphere, culled
//!   per view by frustum, distance and screen size into the same draw lists, opaque instances front to back.
//! - Levels of detail ([`lod`], [`RendererSettings::lod`]): balls and cylinders far away drawn with fewer triangles,
//!   chosen by projected size in the cull with hysteresis, and shadows never finer than what receives them.
//! - Lighting and the sky ([`lighting`], [`Renderer::set_lighting`]): the sun or moon, a hemisphere ambient and an
//!   analytic sky drawn last at the far plane, from the `Lighting` service ([`LightingReader`]).
//! - Sun shadows ([`shadow`], [`RendererSettings::shadows`]): texel-snapped cascaded shadow maps, each culling its
//!   own casters, the far cascades' static casters cached while they stay, with the moving ones drawn on top.
//! - Point and spot lights ([`lights`], [`RendererSettings::lights`]): clustered forward lighting with a per-frame
//!   budget that fades lights in and out by importance, and a cached shadow atlas for lights with `Shadows`.
//! - The debug overlay ([`overlay`], [`Renderer::set_overlay_enabled`]): rectangles and bitmap-font text in the UI
//!   pass, which carries the performance overlay (ADR 0008 §10).

mod context;
pub mod cull;
pub mod depth;
mod error;
mod graph;
pub mod lighting;
pub mod lights;
pub mod lod;
pub mod overlay;
pub mod parts;
mod renderer;
pub mod resolution;
pub mod scene;
pub mod settings;
pub mod shadow;
mod surface;
mod target;
mod timing;
mod upscale;

pub use context::{Capabilities, ContextOptions, MsaaSampleCounts, RenderContext};
pub use cull::{Casters, CullSettings, CullStats, CullView, Frustum, VisibleSet};
pub use error::RenderError;
pub use graph::{AttachmentOps, ColorAttachment, DepthOps, FramePlan, PassId, PassSet, PlannedPass};
pub use lighting::{LightingReader, SceneLighting};
pub use lights::{LightSettings, LightStats};
pub use lod::{LodSettings, MAX_LOD_LEVELS};
pub use overlay::Overlay;
pub use parts::{Camera, CameraReader, DrawStats, MaterialParams, MaterialTable, PartRenderer};
pub use renderer::{Renderer, RendererSettings};
pub use scene::{GpuScene, InstanceScene};
pub use shadow::{CascadeStats, MAX_CASCADES, ShadowSettings, ShadowStats};
pub use surface::{RenderSurface, SurfaceFrame};
pub use target::OffscreenTarget;
pub use timing::{FrameTimings, PassTiming, TIMING_RING_SIZE};

/// Re-exported so callers name exactly the `wgpu` types this crate's API uses.
pub use wgpu;
