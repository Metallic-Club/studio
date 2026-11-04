//! GPU time per pass from timestamp queries, read back without ever waiting for the GPU.
//!
//! Every timed frame writes a timestamp at the start and end of each pass into its own region of one query set,
//! resolves them into a buffer and copies them into one of [`TIMING_RING_SIZE`] small readback buffers. After the
//! submit that buffer is mapped asynchronously (`map_async`). Later frames poll the device without blocking and
//! collect whichever buffers have been mapped since, so results arrive one or more frames late. If every buffer of
//! the ring is still in flight (the GPU is far behind), the frame is simply not timed. The archived renderer read
//! its timestamps back with a blocking `poll(Wait)` every frame; this never does (ADR 0008 §9).
//!
//! Shadow cascades ([`crate::shadow`]) are timed one by one, in the passes that draw their casters; a cascade whose
//! casters were kept in a frame took no GPU time in it. A cached cascade's map is built from its static map and its
//! moving casters in a pass of its own, timed apart. The local lights' shadow atlas ([`crate::lights`]) is one pass, timed
//! in the frames that draw any of its tiles.
//!
//! A render pass that runs several of the frame's passes (merged scene passes, UI in the finish pass; see
//! [`crate::FramePlan`]) still reports each of them where the device writes timestamps inside a pass
//! (`TIMESTAMP_QUERY_INSIDE_PASSES`) and is not tile-based: a timestamp between two of them ends one and starts the
//! next. A tile-based GPU draws a pass tile by tile, each tile running all of its draws, so a timestamp between draws
//! means nothing there; there, and without the feature, the merged pass is reported under its own name.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::PassId;
use crate::shadow::MAX_CASCADES;

/// How many frames of timestamps can be in flight at once. Two frames are queued at most
/// (`desired_maximum_frame_latency = 2`), and a readback is collected on a later frame's poll, so four buffers keep
/// every frame timed in a steady state.
pub const TIMING_RING_SIZE: usize = 4;

/// The most timed passes in a frame: every pass of the frame, every shadow cascade drawn and built, and the shadow
/// atlas.
const MAX_TIMED: usize = PassId::COUNT + 2 * MAX_CASCADES + 1;
/// Two timestamps (start and end) per timed pass.
const QUERIES_PER_FRAME: u32 = 2 * MAX_TIMED as u32;
/// Bytes of resolved timestamps per frame.
const BYTES_PER_FRAME: u64 = QUERIES_PER_FRAME as u64 * 8;
/// Each frame's region of the resolve buffer; resolve offsets must be aligned to this.
const RESOLVE_STRIDE: u64 = BYTES_PER_FRAME.next_multiple_of(wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT);

const _: () = assert!(BYTES_PER_FRAME <= RESOLVE_STRIDE);

/// GPU time of one pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassTiming {
    /// The pass's name ([`PassId::name`]).
    pub pass: &'static str,
    /// Time between the pass's start and end on the GPU, in milliseconds.
    pub gpu_ms: f32,
}

/// The GPU time of every pass of one frame, and of each shadow cascade drawn in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTimings {
    frame: u64,
    passes: [PassTiming; PassId::COUNT],
    len: usize,
    /// Each cascade's time; 0 for a cascade kept that frame.
    cascades: [f32; MAX_CASCADES],
    /// Cascades in use that frame.
    cascade_count: u8,
    /// Bit `i` set when cascade `i` was drawn (and timed) that frame.
    cascades_drawn: u8,
    /// Each cached cascade's build pass; 0 when its map was kept that frame.
    builds: [f32; MAX_CASCADES],
    /// Bit `i` set when cached cascade `i`'s map was built (and timed) that frame.
    cascades_built: u8,
    /// The local lights' shadow atlas pass; 0 when it drew no tile that frame.
    light_shadows: f32,
}

impl FrameTimings {
    /// The index of the frame these timings belong to (see [`crate::Renderer::frame_index`]); usually one or more
    /// frames behind the frame being rendered.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// The timed passes, in frame order (the shadow cascades are apart: [`FrameTimings::cascade`]).
    pub fn passes(&self) -> &[PassTiming] {
        &self.passes[..self.len]
    }

    /// The GPU time of `pass`, if it ran in that frame.
    pub fn pass(&self, pass: PassId) -> Option<f32> {
        self.passes()
            .iter()
            .find(|timing| timing.pass == pass.name())
            .map(|timing| timing.gpu_ms)
    }

    /// How many shadow cascades were in use that frame.
    pub fn cascade_count(&self) -> usize {
        usize::from(self.cascade_count)
    }

    /// The GPU time of shadow cascade `cascade`'s pass: `Some(0.0)` when the cascade was in use but its map was
    /// kept, `None` when it was not in use.
    pub fn cascade(&self, cascade: usize) -> Option<f32> {
        (cascade < self.cascade_count()).then(|| self.cascades[cascade])
    }

    /// Whether shadow cascade `cascade` was drawn that frame.
    pub fn cascade_drawn(&self, cascade: usize) -> bool {
        cascade < MAX_CASCADES && self.cascades_drawn & (1 << cascade) != 0
    }

    /// The GPU time of the pass building cached shadow cascade `cascade`'s map from its static map and its moving
    /// casters: `Some(0.0)` when the cascade was in use but its map was not built that frame, `None` when it was not
    /// in use.
    pub fn cascade_build(&self, cascade: usize) -> Option<f32> {
        (cascade < self.cascade_count()).then(|| self.builds[cascade])
    }

    /// Whether cached shadow cascade `cascade`'s map was built that frame.
    pub fn cascade_built(&self, cascade: usize) -> bool {
        cascade < MAX_CASCADES && self.cascades_built & (1 << cascade) != 0
    }

    /// The GPU time of every shadow pass that frame: the cascades drawn and built.
    pub fn shadow_ms(&self) -> f32 {
        self.cascades.iter().sum::<f32>() + self.builds.iter().sum::<f32>()
    }

    /// The GPU time of the local lights' shadow atlas pass ([`crate::lights`]): 0 in a frame that kept every tile.
    pub fn light_shadow_ms(&self) -> f32 {
        self.light_shadows
    }

    /// The sum of every pass's GPU time, shadows included, in milliseconds.
    pub fn total_ms(&self) -> f32 {
        self.passes().iter().map(|timing| timing.gpu_ms).sum::<f32>() + self.shadow_ms() + self.light_shadows
    }
}

/// A pass a frame timed: one of the frame's passes, a shadow cascade's, or the local lights' shadow atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Timed {
    Pass(PassId),
    Cascade(u8),
    Built(u8),
    LightShadows,
}

/// A readback slot's state, shared with its `map_async` callback.
const FREE: u8 = 0;
const IN_FLIGHT: u8 = 1;
const MAPPED: u8 = 2;
const FAILED: u8 = 3;

struct Slot {
    readback: wgpu::Buffer,
    state: Arc<AtomicU8>,
    frame: u64,
    /// The timed passes in query order.
    timed: [Timed; MAX_TIMED],
    len: usize,
    cascade_count: u8,
}

/// The query set, resolve buffer and readback ring. Only exists when the device has timestamp queries.
pub(crate) struct GpuTimer {
    query_set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: [Slot; TIMING_RING_SIZE],
    /// Nanoseconds per timestamp tick.
    period_ns: f64,
    latest: Option<FrameTimings>,
    /// Each pass a merged render pass runs is timed apart, with timestamps written between them.
    inside_passes: bool,
}

impl GpuTimer {
    /// Creates the timer, or `None` when the device cannot write timestamps around passes. With `inside_passes` (the
    /// device can write them inside a pass, and they mean something there) the passes a merged render pass runs are
    /// timed one by one; otherwise the merged pass is timed as one.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue, inside_passes: bool) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("metallic pass timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERIES_PER_FRAME * TIMING_RING_SIZE as u32,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic timestamp resolve"),
            size: RESOLVE_STRIDE * TIMING_RING_SIZE as u64,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = std::array::from_fn(|_| Slot {
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("metallic timestamp readback"),
                size: BYTES_PER_FRAME,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            state: Arc::new(AtomicU8::new(FREE)),
            frame: 0,
            timed: [Timed::Pass(PassId::Opaque); MAX_TIMED],
            len: 0,
            cascade_count: 0,
        });
        Some(Self {
            query_set,
            resolve,
            slots,
            period_ns: f64::from(queue.get_timestamp_period()),
            latest: None,
            inside_passes: inside_passes
                && device
                    .features()
                    .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
        })
    }

    /// Whether the passes a merged render pass runs are timed one by one.
    pub(crate) fn inside_passes(&self) -> bool {
        self.inside_passes
    }

    /// Collects every readback that finished since the last frame and picks a free slot for this one. Never
    /// blocks: `None` means every slot is still in flight and this frame goes untimed.
    pub(crate) fn begin_frame(&mut self, device: &wgpu::Device) -> Option<usize> {
        // Non-blocking: runs the callbacks of maps that completed, nothing more.
        let _ = device.poll(wgpu::PollType::Poll);
        self.collect();
        self.slots
            .iter()
            .position(|slot| slot.state.load(Ordering::Acquire) == FREE)
    }

    /// The timestamp writes for the `index`-th timed pass of the frame recorded in `slot` (in recording order:
    /// the shadow cascades drawn, then the frame's passes).
    pub(crate) fn pass_writes(&self, slot: usize, index: usize) -> wgpu::RenderPassTimestampWrites<'_> {
        self.span_writes(slot, index, index)
    }

    /// The timestamp writes for a render pass that runs the `first`-th to the `last`-th timed passes of the frame:
    /// the first one's start and the last one's end. [`GpuTimer::write_between`] writes the ones between them.
    pub(crate) fn span_writes(&self, slot: usize, first: usize, last: usize) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(Self::query(slot, first)),
            end_of_pass_write_index: Some(Self::query(slot, last) + 1),
        }
    }

    /// Inside a render pass, ends the `index`-th timed pass of the frame recorded in `slot` and starts the next.
    pub(crate) fn write_between(&self, render_pass: &mut wgpu::RenderPass<'_>, slot: usize, index: usize) {
        let end = Self::query(slot, index) + 1;
        render_pass.write_timestamp(&self.query_set, end);
        render_pass.write_timestamp(&self.query_set, Self::query(slot, index + 1));
    }

    /// The start query of the `index`-th timed pass of `slot`'s frame (its end is the next).
    fn query(slot: usize, index: usize) -> u32 {
        slot as u32 * QUERIES_PER_FRAME + 2 * index.min(MAX_TIMED - 1) as u32
    }

    /// Records which passes frame `frame` timed (`timed`, in the order of their queries) with `cascade_count`
    /// shadow cascades in use, and copies their timestamps towards `slot`'s readback buffer.
    pub(crate) fn end_frame(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
        frame: u64,
        timed: impl Iterator<Item = Timed>,
        cascade_count: u8,
    ) {
        let entry = &mut self.slots[slot];
        entry.frame = frame;
        entry.len = 0;
        entry.cascade_count = cascade_count;
        for pass in timed.take(MAX_TIMED) {
            entry.timed[entry.len] = pass;
            entry.len += 1;
        }
        if entry.len == 0 {
            return;
        }
        let first = slot as u32 * QUERIES_PER_FRAME;
        let offset = slot as u64 * RESOLVE_STRIDE;
        let bytes = entry.len as u64 * 16;
        encoder.resolve_query_set(
            &self.query_set,
            first..first + 2 * entry.len as u32,
            &self.resolve,
            offset,
        );
        encoder.copy_buffer_to_buffer(&self.resolve, offset, &entry.readback, 0, bytes);
        entry.state.store(IN_FLIGHT, Ordering::Release);
    }

    /// Starts the asynchronous readback of `slot`. Call after the frame's commands are submitted.
    pub(crate) fn after_submit(&self, slot: usize) {
        let entry = &self.slots[slot];
        if entry.len == 0 {
            return;
        }
        let state = Arc::clone(&entry.state);
        entry
            .readback
            .map_async(wgpu::MapMode::Read, ..entry.len as u64 * 16, move |result| {
                state.store(if result.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
            });
    }

    /// The newest timings collected so far.
    pub(crate) fn latest(&self) -> Option<&FrameTimings> {
        self.latest.as_ref()
    }

    fn collect(&mut self) {
        for slot in &mut self.slots {
            match slot.state.load(Ordering::Acquire) {
                MAPPED => {
                    let timings = read_slot(slot, self.period_ns);
                    slot.readback.unmap();
                    slot.state.store(FREE, Ordering::Release);
                    if let Some(timings) = timings
                        && self.latest.is_none_or(|latest| latest.frame < timings.frame)
                    {
                        self.latest = Some(timings);
                    }
                }
                // The map failed (device lost or destroyed): nothing is mapped, so the slot is simply reused.
                FAILED => slot.state.store(FREE, Ordering::Release),
                _ => {}
            }
        }
    }
}

fn read_slot(slot: &Slot, period_ns: f64) -> Option<FrameTimings> {
    let view = slot.readback.get_mapped_range(..slot.len as u64 * 16).ok()?;
    let mut timings = FrameTimings {
        frame: slot.frame,
        passes: [PassTiming { pass: "", gpu_ms: 0.0 }; PassId::COUNT],
        len: 0,
        cascades: [0.0; MAX_CASCADES],
        cascade_count: slot.cascade_count,
        cascades_drawn: 0,
        builds: [0.0; MAX_CASCADES],
        cascades_built: 0,
        light_shadows: 0.0,
    };
    let (pairs, _) = view.as_chunks::<16>();
    for (pair, timed) in pairs.iter().zip(&slot.timed[..slot.len]) {
        let (start, end) = pair.split_at(8);
        let (start, end) = (ticks(start), ticks(end));
        // Some drivers (AMD on Vulkan, measured) return zeros for a query set's very first frame, and no clock reads
        // zero once the GPU is running; a frame with a missing or backwards timestamp is dropped, not reported.
        if start == 0 || end < start {
            return None;
        }
        // Precision is lost only past ~2^53 ticks, far beyond any frame.
        let gpu_ms = ((end - start) as f64 * period_ns / 1.0e6) as f32;
        match *timed {
            Timed::Pass(pass) => {
                if timings.len < PassId::COUNT {
                    timings.passes[timings.len] = PassTiming {
                        pass: pass.name(),
                        gpu_ms,
                    };
                    timings.len += 1;
                }
            }
            Timed::Cascade(cascade) => {
                if let Some(ms) = timings.cascades.get_mut(usize::from(cascade)) {
                    *ms = gpu_ms;
                    timings.cascades_drawn |= 1 << cascade;
                }
            }
            Timed::Built(cascade) => {
                if let Some(ms) = timings.builds.get_mut(usize::from(cascade)) {
                    *ms = gpu_ms;
                    timings.cascades_built |= 1 << cascade;
                }
            }
            Timed::LightShadows => timings.light_shadows = gpu_ms,
        }
    }
    Some(timings)
}

fn ticks(bytes: &[u8]) -> u64 {
    let mut raw = [0; 8];
    raw.copy_from_slice(bytes);
    u64::from_le_bytes(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_timings_report_passes_in_order() {
        let mut timings = FrameTimings {
            frame: 7,
            passes: [PassTiming { pass: "", gpu_ms: 0.0 }; PassId::COUNT],
            len: 2,
            cascades: [0.5, 0.0, 0.0, 0.0],
            cascade_count: 2,
            cascades_drawn: 1,
            builds: [0.0, 0.125, 0.0, 0.0],
            cascades_built: 2,
            light_shadows: 0.0,
        };
        timings.passes[0] = PassTiming {
            pass: PassId::Opaque.name(),
            gpu_ms: 1.5,
        };
        timings.passes[1] = PassTiming {
            pass: PassId::Ui.name(),
            gpu_ms: 0.25,
        };
        assert_eq!(timings.frame(), 7);
        assert_eq!(timings.passes().len(), 2);
        assert_eq!(timings.pass(PassId::Ui), Some(0.25));
        assert_eq!(timings.pass(PassId::Sky), None);
        assert_eq!(timings.cascade_count(), 2);
        assert_eq!(
            (timings.cascade(0), timings.cascade(1), timings.cascade(2)),
            (Some(0.5), Some(0.0), None)
        );
        assert!(timings.cascade_drawn(0) && !timings.cascade_drawn(1));
        assert_eq!(
            (timings.cascade_build(1), timings.cascade_build(2)),
            (Some(0.125), None)
        );
        assert!(timings.cascade_built(1) && !timings.cascade_built(0));
        assert!((timings.shadow_ms() - 0.625).abs() < 1e-6);
        assert!((timings.total_ms() - 2.375).abs() < 1e-6);
    }

    #[test]
    fn timestamps_are_little_endian_ticks() {
        assert_eq!(ticks(&1_000u64.to_le_bytes()), 1_000);
    }
}
