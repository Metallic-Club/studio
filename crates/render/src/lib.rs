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

mod context;
pub mod depth;
mod error;
mod graph;
mod renderer;
pub mod scene;
mod surface;
mod target;
mod timing;

pub use context::{Capabilities, ContextOptions, MsaaSampleCounts, RenderContext};
pub use error::RenderError;
pub use graph::{AttachmentOps, ColorAttachment, DepthOps, FramePlan, PassId, PassSet, PlannedPass};
pub use renderer::{Renderer, RendererSettings};
pub use scene::{GpuScene, InstanceScene};
pub use surface::{RenderSurface, SurfaceFrame};
pub use target::OffscreenTarget;
pub use timing::{FrameTimings, PassTiming, TIMING_RING_SIZE};

/// Re-exported so callers name exactly the `wgpu` types this crate's API uses.
pub use wgpu;
