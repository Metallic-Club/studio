//! Large textures made off the frame (#152, ADR 0008 §9): the textures a settings change needs (the sun's shadow
//! maps, the lights' shadow atlas, the scene's attachments) are made one a frame, in the background, while frames
//! keep drawing with the ones they replace; each system switches to its new textures once all of them exist.
//!
//! # Why
//!
//! Drawing a switch's new maps again is cheap (all of Ultra's cascades and atlas tiles take about 0.4 ms of GPU on
//! the dev PC, `docs/perf.md`). Making their memory is not: on DX12 and Vulkan a texture that needs a new device
//! memory block waits on the CPU while the OS clears the block, about 0.12 ms a MiB (15 to 30 ms for wgpu's default
//! blocks of 128 to 256 MiB), and on OpenGL the driver does the same at the texture's first draw. A switch from High to
//! Ultra made 270 MiB at once, a 16 ms frame uncapped and 23 to 27 ms with vsync at 120 Hz.
//!
//! So:
//!
//! - **The device's memory blocks are 128 MiB** ([`crate::RenderContext`] asks for them rather than wgpu's default,
//!   which grows to 256): a new one costs about 15 ms, which the worker below hides; 256 MiB did not always hide.
//! - **One texture a frame**, made on a worker thread that starts right after the frame is submitted
//!   ([`TextureMaker::kick`]): the frame's present waits for the display meanwhile, which hides up to about a 120 Hz
//!   frame of it. A frame that writes a buffer while the worker still makes its texture waits for it (wgpu holds its
//!   allocator's lock), so only one is made at a time. On OpenGL (one context thread) and WebAssembly the texture is
//!   made at the same moment, inline: making it is cheap there.
//! - **Each new texture is cleared in the frame it arrives** ([`TextureMaker::begin_frame`]), so OpenGL's first
//!   use, which is where it pays, falls in its own frame too; no texture is ever cleared again by wgpu's lazy
//!   initialisation.
//! - **Small textures** (under [`BACKGROUND_BYTES`]) and everything made **before a scene's first frame** are made at
//!   once, as before: the first frame shows what it should.
//!
//! The old textures stay in use until the new ones replace them, so for a few frames both exist: at the switch to
//! Ultra on the dev PC the peak is 452 MiB against Ultra's steady 368, for 4 frames (see `docs/perf.md`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Textures of at least this many bytes are made in the background once a scene's first frame is drawn; smaller
/// ones at once (a few milliseconds at most, and usually room in a block that exists).
pub(crate) const BACKGROUND_BYTES: u64 = 16 << 20;

/// What a texture is made from: a descriptor that owns everything it names, so it can be sent to the worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextureSpec {
    pub(crate) label: &'static str,
    pub(crate) size: wgpu::Extent3d,
    pub(crate) samples: u32,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) usage: wgpu::TextureUsages,
}

impl TextureSpec {
    /// A single-sampled 2D texture of `width` × `height` × `layers`.
    pub(crate) fn new(
        label: &'static str,
        (width, height, layers): (u32, u32, u32),
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> Self {
        Self {
            label,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: layers,
            },
            samples: 1,
            format,
            usage,
        }
    }

    /// The same with `samples` per texel.
    pub(crate) fn multisampled(self, samples: u32) -> Self {
        Self { samples, ..self }
    }

    /// Its memory, about (the texels' bytes; drivers add a little for alignment and compression).
    pub(crate) fn bytes(&self) -> u64 {
        let texel = u64::from(self.format.block_copy_size(None).unwrap_or(4));
        u64::from(self.size.width)
            * u64::from(self.size.height)
            * u64::from(self.size.depth_or_array_layers)
            * u64::from(self.samples)
            * texel
    }

    /// Makes it now, on this thread, outside any maker: for placeholders and textures a frame needs at once.
    pub(crate) fn make_now(&self, device: &wgpu::Device) -> wgpu::Texture {
        self.make(device)
    }

    fn make(&self, device: &wgpu::Device) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(self.label),
            size: self.size,
            mip_level_count: 1,
            sample_count: self.samples,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: self.usage,
            view_formats: &[],
        })
    }
}

/// A texture asked for in the background ([`TextureMaker::make`]): [`TextureMaker::take`] hands it over once made.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Ticket(u64);

/// A texture asked for: made already, or coming.
#[derive(Debug)]
pub(crate) enum Made {
    /// Made at once.
    Ready(wgpu::Texture),
    /// Being made in the background.
    Pending(Ticket),
}

/// A texture a settings change needs: made already, or still being made.
#[derive(Debug)]
pub(crate) enum Slot {
    /// Here.
    Made(wgpu::Texture),
    /// Still being made.
    Waiting(Ticket),
}

impl Slot {
    /// The slot of a texture asked for.
    pub(crate) fn new(made: Made) -> Self {
        match made {
            Made::Ready(texture) => Slot::Made(texture),
            Made::Pending(ticket) => Slot::Waiting(ticket),
        }
    }

    /// Takes the texture if it arrived; whether it is here.
    pub(crate) fn poll(&mut self, maker: &TextureMaker) -> bool {
        if let Slot::Waiting(ticket) = self
            && let Some(texture) = maker.take(ticket)
        {
            *self = Slot::Made(texture);
        }
        matches!(self, Slot::Made(_))
    }

    /// The texture, once here.
    pub(crate) fn into_texture(self) -> Option<wgpu::Texture> {
        match self {
            Slot::Made(texture) => Some(texture),
            Slot::Waiting(_) => None,
        }
    }

    /// Gives it up (a newer change replaced the one it was for).
    pub(crate) fn cancel(self, maker: &TextureMaker) {
        if let Slot::Waiting(ticket) = self {
            maker.cancel(ticket);
        }
    }
}

/// What the textures made off the frame have done: for the trace of a switch and reports.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextureStats {
    /// Textures asked for and not yet handed over.
    pub pending: u32,
    /// Textures made in the background so far, and their bytes.
    pub made: u32,
    /// Bytes of the textures made in the background so far.
    pub made_bytes: u64,
    /// The longest one took to make, in milliseconds (on the worker, or inline on OpenGL).
    pub longest_ms: f32,
}

/// A queued texture.
struct Job {
    id: u64,
    spec: TextureSpec,
}

/// The queues, shared by every holder of the maker and its worker.
#[derive(Default)]
struct State {
    /// Before a scene's first frame: everything is made at once.
    immediate: bool,
    /// Asked for, not yet started.
    queued: VecDeque<Job>,
    /// Handed to the worker by [`TextureMaker::kick`], not yet taken by it.
    start: Option<Job>,
    /// The worker is making this one.
    running: Option<u64>,
    /// Made by the worker, not yet collected by a frame.
    made: Option<(u64, wgpu::Texture, Duration)>,
    /// Collected: waiting for its system to take it.
    ready: Vec<(u64, wgpu::Texture)>,
    /// Arrived this frame, to be cleared in it.
    fresh: Vec<wgpu::Texture>,
    /// Given up while being made: dropped when it arrives.
    cancelled: Vec<u64>,
    /// Asked for and not yet handed over (or given up).
    tickets: u32,
    next_id: u64,
    worker: Worker,
    closed: bool,
    stats: TextureStats,
}

impl State {
    /// Nothing asked for, being made or to be cleared.
    fn idle(&self) -> bool {
        self.tickets == 0
            && self.fresh.is_empty()
            && self.start.is_none()
            && self.running.is_none()
            && self.made.is_none()
    }
}

/// Whether the worker thread runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Worker {
    /// Not started yet (it starts with the first texture made in the background).
    #[default]
    Absent,
    Running,
    /// No thread could be started: textures are made inline.
    Failed,
}

struct Shared {
    state: Mutex<State>,
    /// Signalled when the worker has a texture to make (or the maker closes).
    work: Condvar,
    /// Signalled when the worker made one.
    made: Condvar,
    /// Whether anything is asked for, being made or to be cleared ([`State::idle`]): a frame reads only this when
    /// nothing is.
    busy: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Nothing panics while holding the lock (textures are made outside it).
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Notes what `state` now holds, before its lock is let go.
    fn update(&self, state: &mut State) {
        state.stats.pending = state.tickets;
        self.busy.store(!state.idle(), Ordering::Release);
    }
}

/// Makes large textures off the frame (see the [module documentation](self)). The renderer owns it; the systems that
/// make textures on settings changes hold clones.
#[derive(Clone)]
pub(crate) struct TextureMaker {
    device: wgpu::Device,
    shared: Arc<Shared>,
    /// Ends the worker when the last clone goes.
    _closer: Arc<Closer>,
    /// Textures are made on a worker thread (otherwise inline, at the same moment).
    background: bool,
}

impl std::fmt::Debug for TextureMaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextureMaker")
            .field("background", &self.background)
            .field("busy", &self.busy())
            .finish()
    }
}

impl TextureMaker {
    /// A maker for `device`, making everything at once until [`TextureMaker::set_immediate`] says otherwise. With
    /// `background`, textures are made on a worker thread (started with the first one); otherwise inline.
    pub(crate) fn new(device: &wgpu::Device, background: bool) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                immediate: true,
                ..State::default()
            }),
            work: Condvar::new(),
            made: Condvar::new(),
            busy: AtomicBool::new(false),
        });
        Self {
            device: device.clone(),
            _closer: Arc::new(Closer(Arc::clone(&shared))),
            shared,
            background: background && !cfg!(target_family = "wasm"),
        }
    }

    /// Makes everything at once (before and during a scene's first frame), or large textures in the background.
    pub(crate) fn set_immediate(&self, immediate: bool) {
        self.shared.lock().immediate = immediate;
    }

    /// Asks for a texture: made now when it is small or a scene's first frame is still to come, otherwise queued.
    pub(crate) fn make(&self, spec: TextureSpec) -> Made {
        let mut state = self.shared.lock();
        if state.immediate || spec.bytes() < BACKGROUND_BYTES {
            drop(state);
            return Made::Ready(spec.make(&self.device));
        }
        let id = state.next_id;
        state.next_id += 1;
        state.queued.push_back(Job { id, spec });
        state.tickets += 1;
        self.shared.update(&mut state);
        Made::Pending(Ticket(id))
    }

    /// The texture `ticket` asked for, once it arrived (and was cleared in the frame it did); `None` until then.
    pub(crate) fn take(&self, ticket: &Ticket) -> Option<wgpu::Texture> {
        let mut state = self.shared.lock();
        let at = state.ready.iter().position(|(id, _)| *id == ticket.0)?;
        let (_, texture) = state.ready.swap_remove(at);
        state.tickets = state.tickets.saturating_sub(1);
        self.shared.update(&mut state);
        Some(texture)
    }

    /// Gives up `ticket` (a newer settings change replaced what it was for): dropped wherever it is.
    pub(crate) fn cancel(&self, ticket: Ticket) {
        let mut state = self.shared.lock();
        let id = ticket.0;
        if let Some(at) = state.queued.iter().position(|job| job.id == id) {
            state.queued.remove(at);
        } else if let Some(at) = state.ready.iter().position(|(ready, _)| *ready == id) {
            state.ready.swap_remove(at);
        } else if state.start.as_ref().is_some_and(|job| job.id == id) {
            state.start = None;
        } else if state.made.as_ref().is_some_and(|(made, ..)| *made == id) {
            state.made = None;
        } else {
            state.cancelled.push(id);
        }
        state.tickets = state.tickets.saturating_sub(1);
        self.shared.update(&mut state);
    }

    /// Whether anything is asked for, being made or to be cleared. Reads one atomic: nothing else when idle.
    pub(crate) fn busy(&self) -> bool {
        self.shared.busy.load(Ordering::Acquire)
    }

    /// At the start of a frame, before its passes: takes what the worker made since the last frame and records a clear
    /// of it into `encoder` (see [`clear`]), so its systems can take it in this frame. Waits for nothing; with nothing
    /// pending it touches nothing.
    pub(crate) fn begin_frame(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.busy() {
            return;
        }
        let fresh = {
            let mut state = self.shared.lock();
            if let Some((id, texture, time)) = state.made.take() {
                Self::arrived(&mut state, id, texture, time);
            }
            let fresh = std::mem::take(&mut state.fresh);
            self.shared.update(&mut state);
            fresh
        };
        for texture in &fresh {
            clear(encoder, texture);
        }
    }

    fn arrived(state: &mut State, id: u64, texture: wgpu::Texture, time: Duration) {
        state.stats.made += 1;
        state.stats.longest_ms = state.stats.longest_ms.max(time.as_secs_f32() * 1000.0);
        if let Some(at) = state.cancelled.iter().position(|&cancelled| cancelled == id) {
            state.cancelled.swap_remove(at);
            return;
        }
        state.stats.made_bytes += texture_bytes(&texture);
        state.fresh.push(texture.clone());
        state.ready.push((id, texture));
    }

    /// After a frame is submitted: starts making the next queued texture, unless one is still being made or waits to
    /// be collected. On a worker this overlaps the frame's present; inline it is made now.
    pub(crate) fn kick(&self) {
        if !self.busy() {
            return;
        }
        let mut state = self.shared.lock();
        if state.start.is_some() || state.running.is_some() || state.made.is_some() {
            return;
        }
        let Some(job) = state.queued.pop_front() else {
            return;
        };
        // Without a thread (none could be started), inline, as on OpenGL.
        if self.background && self.ensure_worker(&mut state) {
            state.start = Some(job);
            self.shared.update(&mut state);
            self.shared.work.notify_one();
            return;
        }
        drop(state);
        let start = Instant::now();
        let texture = job.spec.make(&self.device);
        let mut state = self.shared.lock();
        state.made = Some((job.id, texture, start.elapsed()));
        self.shared.update(&mut state);
    }

    /// Makes every texture asked for now, waiting for the worker's: for tests, tools and loading screens (with
    /// [`crate::Renderer::wait_for_pipelines`]), never a game's frame. They are handed over (and cleared) with the
    /// next frame.
    pub(crate) fn finish(&self) {
        if !self.busy() {
            return;
        }
        let mut state = self.shared.lock();
        loop {
            if let Some((id, texture, time)) = state.made.take() {
                Self::arrived(&mut state, id, texture, time);
            }
            if state.start.is_none() && state.running.is_none() {
                break;
            }
            state = self
                .shared
                .made
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        let queued: Vec<Job> = state.queued.drain(..).collect();
        self.shared.update(&mut state);
        drop(state);
        for job in queued {
            let start = Instant::now();
            let texture = job.spec.make(&self.device);
            let elapsed = start.elapsed();
            let mut state = self.shared.lock();
            Self::arrived(&mut state, job.id, texture, elapsed);
            self.shared.update(&mut state);
        }
    }

    /// What has been made and what is pending.
    pub(crate) fn stats(&self) -> TextureStats {
        self.shared.lock().stats
    }

    /// Starts the worker unless it runs; `false` when no thread can be started.
    fn ensure_worker(&self, state: &mut State) -> bool {
        match state.worker {
            Worker::Running => return true,
            Worker::Failed => return false,
            Worker::Absent => {}
        }
        let (shared, device) = (Arc::clone(&self.shared), self.device.clone());
        let spawned = std::thread::Builder::new()
            .name("metallic textures".into())
            .spawn(move || work(&shared, &device));
        state.worker = if spawned.is_ok() {
            Worker::Running
        } else {
            Worker::Failed
        };
        state.worker == Worker::Running
    }
}

/// Closes the queue when the last [`TextureMaker`] goes, so the worker ends.
struct Closer(Arc<Shared>);

impl Drop for Closer {
    fn drop(&mut self) {
        self.0.lock().closed = true;
        self.0.work.notify_all();
    }
}

/// The worker: makes the texture [`TextureMaker::kick`] hands it, one at a time, until the maker is dropped.
fn work(shared: &Shared, device: &wgpu::Device) {
    loop {
        let job = {
            let mut state = shared.lock();
            loop {
                if state.closed {
                    return;
                }
                if let Some(job) = state.start.take() {
                    state.running = Some(job.id);
                    break job;
                }
                state = shared.work.wait(state).unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        let start = Instant::now();
        let texture = job.spec.make(device);
        let elapsed = start.elapsed();
        let mut state = shared.lock();
        state.running = None;
        state.made = Some((job.id, texture, elapsed));
        shared.update(&mut state);
        shared.made.notify_all();
    }
}

/// About how many bytes `texture` holds (its texels).
fn texture_bytes(texture: &wgpu::Texture) -> u64 {
    let size = texture.size();
    let texel = u64::from(texture.format().block_copy_size(None).unwrap_or(4));
    u64::from(size.width)
        * u64::from(size.height)
        * u64::from(size.depth_or_array_layers)
        * u64::from(texture.sample_count())
        * texel
}

/// Clears every layer of `texture` in passes of their own: depth to 1, colour to black, stored.
fn clear(encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) {
    // Tile memory only (on tile-based GPUs): never stored, so nothing to clear.
    if texture.usage().contains(wgpu::TextureUsages::TRANSIENT_ATTACHMENT)
        || !texture.usage().contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
    {
        return;
    }
    let depth = texture.format().is_depth_stencil_format();
    for layer in 0..texture.depth_or_array_layers() {
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("metallic new texture"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: layer,
            array_layer_count: Some(1),
            ..wgpu::TextureViewDescriptor::default()
        });
        let color = [Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let depth_stencil = depth.then(|| wgpu::RenderPassDepthStencilAttachment {
            view: &view,
            depth_ops: texture.format().has_depth_aspect().then_some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: texture.format().has_stencil_aspect().then_some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0),
                store: wgpu::StoreOp::Store,
            }),
        });
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("metallic new texture"),
            color_attachments: if depth { &[] } else { &color },
            depth_stencil_attachment: depth_stencil,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_specs_bytes_count_layers_and_samples() {
        let spec = TextureSpec::new(
            "t",
            (3072, 3072, 3),
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        assert_eq!(spec.bytes(), 3072 * 3072 * 3 * 4);
        let msaa = TextureSpec::new(
            "m",
            (1920, 1080, 1),
            wgpu::TextureFormat::Rg11b10Ufloat,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        )
        .multisampled(8);
        assert_eq!(msaa.bytes(), 1920 * 1080 * 8 * 4);
    }
}
