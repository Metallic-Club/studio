//! The Metallic Engine frame profiler: nested CPU scopes per subsystem, time per script and GPU
//! pass timings, kept for the last [`HISTORY`] frames.
//!
//! ```
//! use metallic_profiler::{Profiler, Subsystem};
//! Profiler::set_enabled(true);
//! Profiler::begin_frame();
//! {
//!     let _physics = Profiler::scope(Subsystem::Physics, "physics.step");
//!     std::hint::black_box(2 + 2);
//! }
//! Profiler::end_frame();
//! Profiler::with_frames(|frames| {
//!     let frame = frames.latest().unwrap();
//!     assert_eq!(frame.scopes()[0].name, "physics.step");
//! });
//! ```
//!
//! **One profiler per thread.** The game loop runs on one thread, so the profiler is a
//! thread-local: any crate can open a [`Profiler::scope`] without a handle being passed down.
//! Scopes opened on other threads go to that thread's profiler, which records nothing unless that
//! thread enables it and marks frames.
//!
//! **Zero cost when off.** Disabled (the default), a scope is one thread-local flag read; no clock
//! is read and nothing is stored. Nothing is allocated until the profiler is first enabled.
//!
//! **Bounded.** Enabling reserves the history once ([`HISTORY`] frames of fixed size, about 3 MB);
//! recording a frame never allocates. A frame keeps up to [`MAX_SCOPES`] scopes nested up to
//! [`MAX_DEPTH`] deep (more are counted in [`FrameRecord::dropped_scopes`] and their time stays in
//! their parent), up to [`MAX_SCRIPTS`] scripts (the rest are summed in
//! [`FrameRecord::other_scripts_ns`]) and up to [`MAX_GPU_PASSES`] GPU passes.
//!
//! **GPU time** arrives a few frames late (the GPU finishes after the CPU). The renderer reports
//! it with [`Profiler::record_gpu`] for the frame it was recorded in; until then (or on a device
//! without timestamp queries) [`FrameRecord::gpu`] is `None`, never a made-up zero.
//!
//! Disabling keeps the history, so a host pauses the capture by disabling it and inspects the
//! frames it has.
#![warn(missing_docs)]

mod ring;

pub use ring::RingBuffer;

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::time::Instant;

/// How many frames the profiler keeps (10 s at 60 fps).
pub const HISTORY: usize = 600;
/// The most scopes one frame keeps.
pub const MAX_SCOPES: usize = 128;
/// The deepest nesting one frame keeps.
pub const MAX_DEPTH: usize = 16;
/// The most scripts one frame times one by one.
pub const MAX_SCRIPTS: usize = 32;
/// The most GPU passes one frame keeps.
pub const MAX_GPU_PASSES: usize = 8;

/// The part of the engine a scope's time belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subsystem {
    /// Luau scripts: events, `Heartbeat`, resumed waits.
    Script,
    /// Rigid bodies, movers and the physics sync.
    Physics,
    /// Humanoid characters: movement, touches, body animation, the skinned avatar.
    Character,
    /// Building and submitting the frame's GPU work.
    Render,
    /// In-game GUI and overlays.
    Gui,
    /// Networking (no per-frame network work runs yet).
    Net,
    /// Everything else (camera, input, host work).
    Other,
}

impl Subsystem {
    /// Every subsystem, in display order.
    pub const ALL: [Subsystem; 7] = [
        Subsystem::Script,
        Subsystem::Physics,
        Subsystem::Character,
        Subsystem::Render,
        Subsystem::Gui,
        Subsystem::Net,
        Subsystem::Other,
    ];

    /// A short display name.
    pub fn label(self) -> &'static str {
        match self {
            Subsystem::Script => "Scripts",
            Subsystem::Physics => "Physics",
            Subsystem::Character => "Characters",
            Subsystem::Render => "Render",
            Subsystem::Gui => "GUI",
            Subsystem::Net => "Network",
            Subsystem::Other => "Other",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One closed scope of a frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScopeRecord {
    /// The name given to [`Profiler::scope`].
    pub name: &'static str,
    /// What it is charged to.
    pub subsystem: Subsystem,
    /// 0 for a scope opened directly in the frame, 1 inside another scope, and so on.
    pub depth: u8,
    /// When it opened, in nanoseconds after the frame began.
    pub start_ns: u32,
    /// How long it was open, children included.
    pub inclusive_ns: u32,
    /// The time spent in its recorded child scopes.
    pub children_ns: u32,
}

impl ScopeRecord {
    const EMPTY: ScopeRecord = ScopeRecord {
        name: "",
        subsystem: Subsystem::Other,
        depth: 0,
        start_ns: 0,
        inclusive_ns: 0,
        children_ns: 0,
    };

    /// Its own time: [`Self::inclusive_ns`] less its children.
    pub fn exclusive_ns(&self) -> u32 {
        self.inclusive_ns.saturating_sub(self.children_ns)
    }
}

/// The time one script ran during a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptTime {
    /// The script's Instance id (`InstanceId::to_bits`).
    pub script: u64,
    /// Nanoseconds its code ran, not counting other scripts it called into.
    pub ns: u64,
    /// How many times its code was entered (resumes and event handlers).
    pub calls: u32,
}

impl ScriptTime {
    const EMPTY: ScriptTime = ScriptTime {
        script: 0,
        ns: 0,
        calls: 0,
    };
}

/// The GPU time of one pass (or group of passes) of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuPass {
    /// What ran (for example `"shadows"`).
    pub name: &'static str,
    /// Nanoseconds between its start and end timestamps.
    pub ns: u64,
}

impl GpuPass {
    const EMPTY: GpuPass = GpuPass { name: "", ns: 0 };
}

/// Everything recorded for one frame. Fixed size: copying or storing one never allocates.
#[derive(Debug, Clone, Copy)]
pub struct FrameRecord {
    index: u64,
    frame_ns: u64,
    scopes: [ScopeRecord; MAX_SCOPES],
    scope_count: u16,
    dropped_scopes: u32,
    scripts: [ScriptTime; MAX_SCRIPTS],
    script_count: u8,
    other_scripts_ns: u64,
    gpu: [GpuPass; MAX_GPU_PASSES],
    /// `None` until the renderer reports this frame's GPU time.
    gpu_count: Option<u8>,
}

impl FrameRecord {
    const EMPTY: FrameRecord = FrameRecord {
        index: 0,
        frame_ns: 0,
        scopes: [ScopeRecord::EMPTY; MAX_SCOPES],
        scope_count: 0,
        dropped_scopes: 0,
        scripts: [ScriptTime::EMPTY; MAX_SCRIPTS],
        script_count: 0,
        other_scripts_ns: 0,
        gpu: [GpuPass::EMPTY; MAX_GPU_PASSES],
        gpu_count: None,
    };

    /// The frame's number, counting from 0 at the first frame recorded on this thread.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Wall time from [`Profiler::begin_frame`] to [`Profiler::end_frame`], in nanoseconds.
    pub fn frame_ns(&self) -> u64 {
        self.frame_ns
    }

    /// The recorded scopes, in the order they opened (a parent before its children).
    pub fn scopes(&self) -> &[ScopeRecord] {
        &self.scopes[..self.scope_count as usize]
    }

    /// Scopes that were opened but not kept (too many or too deep).
    pub fn dropped_scopes(&self) -> u32 {
        self.dropped_scopes
    }

    /// The time per script, in the order they first ran this frame.
    pub fn scripts(&self) -> &[ScriptTime] {
        &self.scripts[..self.script_count as usize]
    }

    /// The time of the scripts past [`MAX_SCRIPTS`], summed.
    pub fn other_scripts_ns(&self) -> u64 {
        self.other_scripts_ns
    }

    /// All script time of the frame (the listed scripts and the rest).
    pub fn script_ns(&self) -> u64 {
        self.scripts().iter().map(|s| s.ns).sum::<u64>() + self.other_scripts_ns
    }

    /// The GPU passes, once the renderer has reported them; `None` before that or on a device
    /// without timestamp queries.
    pub fn gpu(&self) -> Option<&[GpuPass]> {
        self.gpu_count.map(|n| &self.gpu[..n as usize])
    }

    /// The frame's total GPU time, if measured.
    pub fn gpu_ns(&self) -> Option<u64> {
        self.gpu().map(|passes| passes.iter().map(|p| p.ns).sum())
    }

    /// The exclusive time of every scope charged to `subsystem`: nested scopes count once, to
    /// the innermost one.
    pub fn subsystem_ns(&self, subsystem: Subsystem) -> u64 {
        self.subsystem_totals()[subsystem.index()]
    }

    /// [`Self::subsystem_ns`] for every subsystem, indexed like [`Subsystem::ALL`].
    pub fn subsystem_totals(&self) -> [u64; Subsystem::ALL.len()] {
        let mut totals = [0u64; Subsystem::ALL.len()];
        for scope in self.scopes() {
            totals[scope.subsystem.index()] += u64::from(scope.exclusive_ns());
        }
        totals
    }

    /// Frame time outside every top-level scope (host work that was not scoped).
    pub fn unscoped_ns(&self) -> u64 {
        let scoped: u64 = self
            .scopes()
            .iter()
            .filter(|s| s.depth == 0)
            .map(|s| u64::from(s.inclusive_ns))
            .sum();
        self.frame_ns.saturating_sub(scoped)
    }
}

/// An open scope; it closes when dropped. Not `Send`: it belongs to the thread that opened it.
#[must_use = "a scope measures until it is dropped"]
pub struct ScopeGuard {
    /// The frame and slot it records into, or `None` when nothing is recorded.
    slot: Option<(u64, u16)>,
    _not_send: PhantomData<*const ()>,
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        if let Some((frame, slot)) = self.slot {
            STATE.with(|state| state.borrow_mut().close(frame, slot));
        }
    }
}

/// Marks one frame: begun by [`Profiler::frame`], ended when dropped. Not `Send`.
#[must_use = "the frame ends when this is dropped"]
pub struct FrameGuard {
    _not_send: PhantomData<*const ()>,
}

impl Drop for FrameGuard {
    fn drop(&mut self) {
        Profiler::end_frame();
    }
}

/// Marks a scope that was opened but not kept (the frame was full or too deep).
const NOT_KEPT: u16 = u16::MAX;

struct State {
    enabled: bool,
    in_frame: bool,
    frame_start: Instant,
    next_index: u64,
    current: Box<FrameRecord>,
    /// The slots of the open scopes, outermost first.
    stack: [u16; MAX_DEPTH],
    depth: usize,
    /// Open scopes past [`MAX_DEPTH`].
    overflow: usize,
    /// A frame that ended while the history was being read ([`Profiler::with_frames`]); it
    /// joins the history when the read ends.
    held: Box<FrameRecord>,
    held_valid: bool,
    /// [`Profiler::clear`] was called while the history was being read.
    clear_pending: bool,
}

impl State {
    fn new() -> State {
        State {
            enabled: false,
            in_frame: false,
            frame_start: Instant::now(),
            next_index: 0,
            current: Box::new(FrameRecord::EMPTY),
            stack: [0; MAX_DEPTH],
            depth: 0,
            overflow: 0,
            held: Box::new(FrameRecord::EMPTY),
            held_valid: false,
            clear_pending: false,
        }
    }

    fn ns_since_start(&self, at: Instant) -> u64 {
        at.saturating_duration_since(self.frame_start).as_nanos() as u64
    }

    fn open(&mut self, subsystem: Subsystem, name: &'static str) -> Option<(u64, u16)> {
        let frame = self.current.index;
        let count = self.current.scope_count as usize;
        if self.depth >= MAX_DEPTH || count >= MAX_SCOPES || self.overflow > 0 {
            self.current.dropped_scopes = self.current.dropped_scopes.saturating_add(1);
            self.overflow += 1;
            return Some((frame, NOT_KEPT));
        }
        let start = self.ns_since_start(Instant::now());
        self.current.scopes[count] = ScopeRecord {
            name,
            subsystem,
            depth: self.depth as u8,
            start_ns: clamp_u32(start),
            inclusive_ns: 0,
            children_ns: 0,
        };
        self.current.scope_count += 1;
        self.stack[self.depth] = count as u16;
        self.depth += 1;
        Some((frame, count as u16))
    }

    fn close(&mut self, frame: u64, slot: u16) {
        if !self.in_frame || frame != self.current.index {
            return;
        }
        if slot == NOT_KEPT {
            self.overflow = self.overflow.saturating_sub(1);
            return;
        }
        let now = self.ns_since_start(Instant::now());
        self.close_slot(slot, now);
    }

    /// Closes `slot` at `now` (and any scope still open inside it, should guards be dropped out
    /// of order).
    fn close_slot(&mut self, slot: u16, now: u64) {
        let Some(pos) = self.stack[..self.depth].iter().rposition(|&s| s == slot) else {
            return;
        };
        while self.depth > pos {
            self.depth -= 1;
            let closing = self.stack[self.depth] as usize;
            let record = &mut self.current.scopes[closing];
            let inclusive = clamp_u32(now.saturating_sub(u64::from(record.start_ns)));
            record.inclusive_ns = inclusive;
            if self.depth > 0 {
                let parent = &mut self.current.scopes[self.stack[self.depth - 1] as usize];
                parent.children_ns = parent.children_ns.saturating_add(inclusive);
            }
        }
    }

    fn begin_frame(&mut self) {
        if !self.enabled {
            return;
        }
        if self.in_frame {
            self.end_frame();
        }
        let index = self.next_index;
        self.next_index += 1;
        let current = &mut *self.current;
        current.index = index;
        current.frame_ns = 0;
        current.scope_count = 0;
        current.dropped_scopes = 0;
        current.script_count = 0;
        current.other_scripts_ns = 0;
        current.gpu_count = None;
        self.depth = 0;
        self.overflow = 0;
        self.in_frame = true;
        self.frame_start = Instant::now();
    }

    fn end_frame(&mut self) {
        if !self.in_frame {
            return;
        }
        let now = self.ns_since_start(Instant::now());
        if self.depth > 0 {
            let outermost = self.stack[0];
            self.close_slot(outermost, now);
        }
        self.current.frame_ns = now;
        self.in_frame = false;
        *self.held = *self.current;
        self.held_valid = true;
    }

    fn charge_script(&mut self, script: u64, ns: u64) {
        let current = &mut *self.current;
        let count = current.script_count as usize;
        if let Some(entry) = current.scripts[..count]
            .iter_mut()
            .find(|s| s.script == script)
        {
            entry.ns += ns;
            entry.calls = entry.calls.saturating_add(1);
        } else if count < MAX_SCRIPTS {
            current.scripts[count] = ScriptTime {
                script,
                ns,
                calls: 1,
            };
            current.script_count += 1;
        } else {
            current.other_scripts_ns += ns;
        }
    }

    /// Stores GPU passes on the open or held frame; false when `frame` is neither.
    fn record_gpu(&mut self, frame: u64, passes: &[GpuPass]) -> bool {
        if self.in_frame && self.current.index == frame {
            set_gpu(&mut self.current, passes);
            true
        } else if self.held_valid && self.held.index == frame {
            set_gpu(&mut self.held, passes);
            true
        } else {
            false
        }
    }
}

fn set_gpu(record: &mut FrameRecord, passes: &[GpuPass]) {
    let n = passes.len().min(MAX_GPU_PASSES);
    record.gpu[..n].copy_from_slice(&passes[..n]);
    record.gpu_count = Some(n as u8);
}

/// Moves the held frame (and a pending clear or reservation) into the history, unless the
/// history is being read right now; then it waits for the next call.
fn flush() {
    FRAMES.with(|frames| {
        let Ok(mut frames) = frames.try_borrow_mut() else {
            return;
        };
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.enabled && frames.capacity() < HISTORY {
                *frames = RingBuffer::new(HISTORY);
            }
            if std::mem::take(&mut state.clear_pending) {
                frames.clear();
            }
            if std::mem::take(&mut state.held_valid) {
                frames.push(*state.held);
            }
        });
    });
}

fn clamp_u32(ns: u64) -> u32 {
    u32::try_from(ns).unwrap_or(u32::MAX)
}

thread_local! {
    /// True while this thread's profiler is enabled and inside a frame: the only thing a
    /// disabled scope reads.
    static RECORDING: Cell<bool> = const { Cell::new(false) };
    /// True while enabled: lets a disabled host's frame marks skip the state.
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static STATE: RefCell<State> = RefCell::new(State::new());
    /// The history, apart from the recording state, so a reader ([`Profiler::with_frames`]) can
    /// call any other profiler function.
    static FRAMES: RefCell<RingBuffer<FrameRecord>> = RefCell::new(RingBuffer::new(0));
}

/// The calling thread's profiler. All functions act on the calling thread only.
pub struct Profiler;

impl Profiler {
    /// Switches recording on or off. The first time it is switched on the history is reserved
    /// (see the crate documentation); switching off ends nothing and keeps the history, and a
    /// frame that was open is dropped.
    pub fn set_enabled(enabled: bool) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if !enabled {
                state.in_frame = false;
                state.depth = 0;
                state.overflow = 0;
            }
            state.enabled = enabled;
        });
        ENABLED.with(|e| e.set(enabled));
        RECORDING.with(|r| r.set(false));
        // Reserves the history now (or after a read that is in progress).
        flush();
    }

    /// True while recording is switched on.
    pub fn is_enabled() -> bool {
        ENABLED.with(Cell::get)
    }

    /// True while enabled and inside a frame: what a scope checks. Cheap.
    #[inline]
    pub fn is_recording() -> bool {
        RECORDING.with(Cell::get)
    }

    /// Starts a frame (ending the previous one if it is still open). Does nothing while disabled.
    pub fn begin_frame() {
        if !Self::is_enabled() {
            return;
        }
        let recording = STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.begin_frame();
            state.in_frame
        });
        RECORDING.with(|r| r.set(recording));
    }

    /// [`Profiler::begin_frame`] now and [`Profiler::end_frame`] when the guard drops, so a host
    /// with early returns ends every frame it begins.
    pub fn frame() -> FrameGuard {
        Self::begin_frame();
        FrameGuard {
            _not_send: PhantomData,
        }
    }

    /// Ends the frame: scopes still open close now, and the frame joins the history.
    pub fn end_frame() {
        if !Self::is_recording() {
            return;
        }
        STATE.with(|state| state.borrow_mut().end_frame());
        RECORDING.with(|r| r.set(false));
        flush();
    }

    /// The number of the frame being recorded, if one is.
    pub fn current_frame() -> Option<u64> {
        if !Self::is_recording() {
            return None;
        }
        STATE.with(|state| Some(state.borrow().current.index))
    }

    /// Opens a scope named `name` charged to `subsystem`; it closes when the guard drops. Scopes
    /// nest: a scope opened while another is open is its child.
    #[inline]
    pub fn scope(subsystem: Subsystem, name: &'static str) -> ScopeGuard {
        let slot = if Self::is_recording() {
            STATE.with(|state| state.borrow_mut().open(subsystem, name))
        } else {
            None
        };
        ScopeGuard {
            slot,
            _not_send: PhantomData,
        }
    }

    /// Adds `ns` nanoseconds of run time to `script` (its `InstanceId::to_bits`) in this frame.
    /// The script host calls it; does nothing unless recording.
    pub fn charge_script(script: u64, ns: u64) {
        if !Self::is_recording() {
            return;
        }
        STATE.with(|state| state.borrow_mut().charge_script(script, ns));
    }

    /// Stores the GPU passes measured for `frame` (a [`Profiler::current_frame`] value), if that
    /// frame is still being recorded or among the newest in the history. Extra passes past
    /// [`MAX_GPU_PASSES`] are dropped.
    pub fn record_gpu(frame: u64, passes: &[GpuPass]) {
        if !Self::is_enabled() {
            return;
        }
        if STATE.with(|state| state.borrow_mut().record_gpu(frame, passes)) {
            return;
        }
        FRAMES.with(|frames| {
            // While the history is being read, the result is dropped.
            if let Ok(mut frames) = frames.try_borrow_mut()
                && let Some(record) = frames.newest_first_mut().take(8).find(|f| f.index == frame)
            {
                set_gpu(record, passes);
            }
        });
    }

    /// Runs `f` with the recorded frames, oldest first. `f` may call any profiler function: a
    /// frame that ends inside it joins the history after it returns (only the last one, if
    /// several end inside it), and GPU results for older frames reported inside it are dropped.
    pub fn with_frames<R>(f: impl FnOnce(&RingBuffer<FrameRecord>) -> R) -> R {
        let out = FRAMES.with(|frames| f(&frames.borrow()));
        flush();
        out
    }

    /// A copy of the newest recorded frame.
    pub fn latest() -> Option<FrameRecord> {
        Self::with_frames(|frames| frames.latest().copied())
    }

    /// Forgets the recorded frames (the storage is kept).
    pub fn clear() {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.clear_pending = true;
            // A frame that ended but is not in the history yet goes too.
            state.held_valid = false;
        });
        flush();
    }
}
