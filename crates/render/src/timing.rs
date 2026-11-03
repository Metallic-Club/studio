//! GPU time per pass from timestamp queries, read back without ever waiting for the GPU.
//!
//! Every timed frame writes a timestamp at the start and end of each pass into its own region of one query set,
//! resolves them into a buffer and copies them into one of [`TIMING_RING_SIZE`] small readback buffers. After the
//! submit that buffer is mapped asynchronously (`map_async`). Later frames poll the device without blocking and
//! collect whichever buffers have been mapped since, so results arrive one or more frames late. If every buffer of
//! the ring is still in flight (the GPU is far behind), the frame is simply not timed. The archived renderer read
//! its timestamps back with a blocking `poll(Wait)` every frame; this never does (ADR 0008 §9).

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::PassId;

/// How many frames of timestamps can be in flight at once. Two frames are queued at most
/// (`desired_maximum_frame_latency = 2`), and a readback is collected on a later frame's poll, so four buffers keep
/// every frame timed in a steady state.
pub const TIMING_RING_SIZE: usize = 4;

/// Two timestamps (start and end) per pass.
const QUERIES_PER_FRAME: u32 = 2 * PassId::COUNT as u32;
/// Bytes of resolved timestamps per frame.
const BYTES_PER_FRAME: u64 = QUERIES_PER_FRAME as u64 * 8;
/// Each frame's region of the resolve buffer; resolve offsets must be aligned to this.
const RESOLVE_STRIDE: u64 = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;

const _: () = assert!(BYTES_PER_FRAME <= RESOLVE_STRIDE);

/// GPU time of one pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassTiming {
    /// The pass's name ([`PassId::name`]).
    pub pass: &'static str,
    /// Time between the pass's start and end on the GPU, in milliseconds.
    pub gpu_ms: f32,
}

/// The GPU time of every pass of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTimings {
    frame: u64,
    passes: [PassTiming; PassId::COUNT],
    len: usize,
}

impl FrameTimings {
    /// The index of the frame these timings belong to (see [`crate::Renderer::frame_index`]); usually one or more
    /// frames behind the frame being rendered.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// The timed passes, in frame order.
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

    /// The sum of every pass's GPU time, in milliseconds.
    pub fn total_ms(&self) -> f32 {
        self.passes().iter().map(|timing| timing.gpu_ms).sum()
    }
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
    passes: [PassId; PassId::COUNT],
    len: usize,
}

/// The query set, resolve buffer and readback ring. Only exists when the device has timestamp queries.
pub(crate) struct GpuTimer {
    query_set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: [Slot; TIMING_RING_SIZE],
    /// Nanoseconds per timestamp tick.
    period_ns: f64,
    latest: Option<FrameTimings>,
}

impl GpuTimer {
    /// Creates the timer, or `None` when the device cannot write timestamps around passes.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
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
            passes: [PassId::Opaque; PassId::COUNT],
            len: 0,
        });
        Some(Self {
            query_set,
            resolve,
            slots,
            period_ns: f64::from(queue.get_timestamp_period()),
            latest: None,
        })
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

    /// The timestamp writes for the `index`-th pass of the frame recorded in `slot`.
    pub(crate) fn pass_writes(&self, slot: usize, index: usize) -> wgpu::RenderPassTimestampWrites<'_> {
        let base = slot as u32 * QUERIES_PER_FRAME + 2 * index as u32;
        wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(base),
            end_of_pass_write_index: Some(base + 1),
        }
    }

    /// Records which passes frame `frame` timed and copies their timestamps towards `slot`'s readback buffer.
    pub(crate) fn end_frame(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
        frame: u64,
        passes: impl Iterator<Item = PassId>,
    ) {
        let entry = &mut self.slots[slot];
        entry.frame = frame;
        entry.len = 0;
        for pass in passes.take(PassId::COUNT) {
            entry.passes[entry.len] = pass;
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
        len: slot.len,
    };
    let (pairs, _) = view.as_chunks::<16>();
    for (i, (pair, pass)) in pairs.iter().zip(&slot.passes[..slot.len]).enumerate() {
        let (start, end) = pair.split_at(8);
        let (start, end) = (ticks(start), ticks(end));
        // Some drivers (AMD on Vulkan, measured) return zeros for a query set's very first frame, and no clock reads
        // zero once the GPU is running; a frame with a missing or backwards timestamp is dropped, not reported.
        if start == 0 || end < start {
            return None;
        }
        let ticks = end - start;
        timings.passes[i] = PassTiming {
            pass: pass.name(),
            // Precision is lost only past ~2^53 ticks, far beyond any frame.
            gpu_ms: (ticks as f64 * period_ns / 1.0e6) as f32,
        };
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
        assert!((timings.total_ms() - 1.75).abs() < 1e-6);
    }

    #[test]
    fn timestamps_are_little_endian_ticks() {
        assert_eq!(ticks(&1_000u64.to_le_bytes()), 1_000);
    }
}
