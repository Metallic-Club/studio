//! GPU time per frame stage, from timestamp queries, for the profiler (#23).
//!
//! Only on a device with [`wgpu::Features::TIMESTAMP_QUERY`] and only while the profiler records
//! a frame; otherwise nothing is created or recorded. A timestamp is written between the stages
//! by an empty compute pass (pass timestamps need no extra feature, unlike timestamps inside an
//! encoder). The results are copied into one of a few small readback buffers and read once
//! the GPU is done, a few frames later, without ever waiting: a frame whose buffers are all
//! still in flight is not timed. They go to [`Profiler::record_gpu`] for the frame they
//! measured.
use metallic_profiler::{GpuPass, Profiler};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// The stages between the timestamps, in frame order.
pub(crate) const STAGES: [&str; 4] = ["shadows", "scene", "exposure", "post"];
/// One timestamp before each stage and one after the last.
const MARKS: u32 = STAGES.len() as u32 + 1;
/// Frames that can be in flight at once.
const SLOTS: usize = 3;
const BYTES: u64 = MARKS as u64 * 8;

const IDLE: u8 = 0;
const MAPPING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    frame: u64,
}

/// Timestamp queries and their readback buffers.
pub(crate) struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: [Slot; SLOTS],
    period_ns: f32,
    /// The slot of the frame being recorded now.
    active: Option<usize>,
}

impl GpuTimer {
    /// A timer, if `device` has timestamp queries.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<GpuTimer> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("metallic gpu timer"),
            ty: wgpu::QueryType::Timestamp,
            count: MARKS,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("metallic gpu timer resolve"),
            size: BYTES,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slot = || Slot {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("metallic gpu timer readback"),
                size: BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            state: Arc::new(AtomicU8::new(IDLE)),
            frame: 0,
        };
        Some(GpuTimer {
            queries,
            resolve,
            slots: [slot(), slot(), slot()],
            period_ns: queue.get_timestamp_period(),
            active: None,
        })
    }

    /// Hands finished results to the profiler, then starts timing `frame` if a readback buffer
    /// is free. Returns whether this frame is timed.
    pub(crate) fn begin(&mut self, device: &wgpu::Device, frame: u64) -> bool {
        self.collect(device);
        self.active = self
            .slots
            .iter()
            .position(|s| s.state.load(Ordering::Acquire) == IDLE);
        if let Some(i) = self.active {
            self.slots[i].frame = frame;
        }
        self.active.is_some()
    }

    /// Writes timestamp `index` (0 before the first stage, `STAGES.len()` after the last).
    pub(crate) fn mark(&self, encoder: &mut wgpu::CommandEncoder, index: u32) {
        if self.active.is_none() {
            return;
        }
        let _ = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("metallic gpu timestamp"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: &self.queries,
                beginning_of_pass_write_index: Some(index),
                end_of_pass_write_index: None,
            }),
        });
    }

    /// Copies this frame's timestamps toward its readback buffer (before the encoder finishes).
    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(i) = self.active else {
            return;
        };
        encoder.resolve_query_set(&self.queries, 0..MARKS, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.slots[i].buffer, 0, BYTES);
    }

    /// Asks for this frame's readback buffer once its commands are submitted.
    pub(crate) fn submitted(&mut self) {
        let Some(i) = self.active.take() else {
            return;
        };
        let slot = &self.slots[i];
        slot.state.store(MAPPING, Ordering::Release);
        let state = slot.state.clone();
        slot.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let next = if result.is_ok() { READY } else { FAILED };
                state.store(next, Ordering::Release);
            });
    }

    /// Forgets the frame being timed (it will not be submitted).
    fn abort(&mut self) {
        self.active = None;
    }

    /// True while a frame is being timed.
    #[cfg(test)]
    fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Readback buffers waiting for the GPU.
    #[cfg(test)]
    fn in_flight(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.state.load(Ordering::Acquire) == MAPPING)
            .count()
    }

    /// Reads every buffer the GPU has finished with (never waits).
    fn collect(&mut self, device: &wgpu::Device) {
        let mapping = self
            .slots
            .iter()
            .any(|s| s.state.load(Ordering::Acquire) == MAPPING);
        if mapping {
            let _ = device.poll(wgpu::PollType::Poll);
        }
        for slot in &self.slots {
            match slot.state.load(Ordering::Acquire) {
                READY => {
                    let mut ticks = [0u64; MARKS as usize];
                    let read = match slot.buffer.slice(..).get_mapped_range() {
                        Ok(view) => {
                            for (t, bytes) in ticks.iter_mut().zip(view.as_chunks::<8>().0) {
                                *t = u64::from_le_bytes(*bytes);
                            }
                            true
                        }
                        Err(_) => false,
                    };
                    slot.buffer.unmap();
                    slot.state.store(IDLE, Ordering::Release);
                    if read {
                        let passes = stage_times(&ticks, self.period_ns);
                        Profiler::record_gpu(slot.frame, &passes);
                    }
                }
                FAILED => slot.state.store(IDLE, Ordering::Release),
                _ => {}
            }
        }
    }
}

/// The timing of one drawn frame. Ends the timer's frame on every way out: a frame that is not
/// [`GpuFrame::submitted`] (an early return) is aborted when this drops, so its timestamps can
/// never be read back as another frame's.
pub(crate) struct GpuFrame<'a> {
    timer: Option<&'a mut GpuTimer>,
    timed: bool,
    done: bool,
}

impl<'a> GpuFrame<'a> {
    /// Starts timing `frame` (a profiler frame number; `None` times nothing). Anything left
    /// over from an earlier frame is forgotten first.
    pub(crate) fn start(
        timer: Option<&'a mut GpuTimer>,
        device: &wgpu::Device,
        frame: Option<u64>,
    ) -> GpuFrame<'a> {
        let mut timed = false;
        let mut timer = timer;
        if let Some(t) = timer.as_deref_mut() {
            t.abort();
            if let Some(frame) = frame {
                timed = t.begin(device, frame);
            }
        }
        GpuFrame {
            timer,
            timed,
            done: false,
        }
    }

    /// True when this frame writes timestamps.
    #[cfg(test)]
    pub(crate) fn is_timed(&self) -> bool {
        self.timed
    }

    /// Writes timestamp `index` (see [`GpuTimer::mark`]).
    pub(crate) fn mark(&self, encoder: &mut wgpu::CommandEncoder, index: u32) {
        if let (true, Some(t)) = (self.timed, self.timer.as_deref()) {
            t.mark(encoder, index);
        }
    }

    /// Copies the timestamps toward the readback buffer (before the encoder finishes).
    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        if let (true, Some(t)) = (self.timed, self.timer.as_deref()) {
            t.resolve(encoder);
        }
    }

    /// The frame's commands were submitted: read its timestamps back.
    pub(crate) fn submitted(mut self) {
        if let (true, Some(t)) = (self.timed, self.timer.as_deref_mut()) {
            t.submitted();
        }
        self.done = true;
    }
}

impl Drop for GpuFrame<'_> {
    fn drop(&mut self) {
        if !self.done
            && let Some(t) = self.timer.as_deref_mut()
        {
            t.abort();
        }
    }
}

/// The time of each stage from the timestamps around it (`period_ns` per tick). A stage whose
/// end reads earlier than its start (a driver reordering timestamps) counts as 0.
pub(crate) fn stage_times(
    ticks: &[u64; MARKS as usize],
    period_ns: f32,
) -> [GpuPass; STAGES.len()] {
    let mut passes = [GpuPass { name: "", ns: 0 }; STAGES.len()];
    for (i, pass) in passes.iter_mut().enumerate() {
        let delta = ticks[i + 1].saturating_sub(ticks[i]);
        *pass = GpuPass {
            name: STAGES[i],
            ns: (delta as f64 * f64::from(period_ns)) as u64,
        };
    }
    passes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RenderContext;

    #[test]
    fn a_frame_that_returns_early_is_never_read_back() {
        let Ok(ctx) = RenderContext::new_headless() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        let Some(mut timer) = GpuTimer::new(&ctx.device, &ctx.queue) else {
            eprintln!("no timestamp queries; skipping");
            return;
        };
        {
            // Begun for frame 7, then the renderer returns before submitting.
            let frame = GpuFrame::start(Some(&mut timer), &ctx.device, Some(7));
            assert!(frame.is_timed());
        }
        assert!(!timer.is_active(), "an aborted frame stayed active");

        // The next frame is not profiled: it must write and read back nothing.
        let frame = GpuFrame::start(Some(&mut timer), &ctx.device, None);
        assert!(!frame.is_timed());
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        frame.mark(&mut encoder, 0);
        frame.resolve(&mut encoder);
        ctx.queue.submit(Some(encoder.finish()));
        frame.submitted();
        assert_eq!(timer.in_flight(), 0, "an unprofiled frame was read back");

        // A profiled frame that is submitted is read back as its own frame.
        let frame = GpuFrame::start(Some(&mut timer), &ctx.device, Some(9));
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        for i in 0..MARKS {
            frame.mark(&mut encoder, i);
        }
        frame.resolve(&mut encoder);
        ctx.queue.submit(Some(encoder.finish()));
        frame.submitted();
        assert_eq!(timer.in_flight(), 1);
        assert!(timer.slots.iter().any(|s| s.frame == 9));
        assert!(!timer.is_active());
    }

    #[test]
    fn stages_are_the_gaps_between_timestamps() {
        let passes = stage_times(&[100, 150, 400, 400, 390], 2.0);
        let ns: Vec<_> = passes.iter().map(|p| (p.name, p.ns)).collect();
        assert_eq!(
            ns,
            [
                ("shadows", 100),
                ("scene", 500),
                ("exposure", 0),
                ("post", 0)
            ]
        );
    }
}
