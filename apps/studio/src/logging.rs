//! Studio's log: what Studio, the engine and the game's scripts write, kept for the Output panel and copied to
//! stderr.
//!
//! - **The engine and Studio** log through the `log` facade. [`install`] makes Studio's logger the process's: it
//!   keeps every line from Metallic's own crates at `info` and above (dependencies from `warn`, because they are
//!   chatty) in the [`global`] [`LogBuffer`], and writes the level `METALLIC_LOG` names (`warn` by default) to stderr.
//! - **Scripts** print through the engine's `OutputSink`: [`ScriptOutput`] puts their lines in the same buffer, so a
//!   game run from Studio prints to its Output panel.
//!
//! The buffer is a ring of the last [`LogBuffer::CAPACITY`] lines, each numbered. A reader (the Output panel) copies
//! the lines after the last number it saw, so it never holds the lock while it draws, and a line logged while it
//! draws simply waits for the next frame.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use log::{Level, LevelFilter};
use metallic_engine::metallic_script::{OutputSink, OutputSource};

/// Where a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Studio or the engine, through `log`.
    Engine,
    /// A game's script.
    Script,
}

/// One line of the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// Its number: every line gets the next one, from 1.
    pub seq: u64,
    /// How serious it is. A script's print is `Info`.
    pub level: Level,
    /// The engine's module (`metallic_project::scene`) or the script's name and line (`Workspace.Door.Script:12`).
    pub source: String,
    /// What it says.
    pub message: String,
    /// The engine or a script.
    pub origin: Origin,
    /// When it was logged (#502).
    pub time: std::time::SystemTime,
}

/// The last lines logged, shared by every thread that logs and every panel that reads them.
#[derive(Debug, Default)]
pub struct LogBuffer {
    inner: Mutex<Ring>,
}

#[derive(Debug, Default)]
struct Ring {
    lines: VecDeque<LogLine>,
    /// The number the next line gets, less one.
    last: u64,
}

impl LogBuffer {
    /// How many lines are kept; older ones are dropped.
    pub const CAPACITY: usize = 5000;

    /// An empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a line.
    pub fn push(&self, level: Level, source: impl Into<String>, message: impl Into<String>, origin: Origin) {
        // A thread that panicked while logging leaves whole lines behind: the ring is still good.
        let mut ring = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        ring.last += 1;
        let line = LogLine {
            seq: ring.last,
            level,
            source: source.into(),
            message: message.into(),
            origin,
            time: std::time::SystemTime::now(),
        };
        if ring.lines.len() == Self::CAPACITY {
            ring.lines.pop_front();
        }
        ring.lines.push_back(line);
    }

    /// Copies the lines numbered after `seen` into `out`, and returns the last number now (pass it next time).
    pub fn read_since(&self, seen: u64, out: &mut impl Extend<LogLine>) -> u64 {
        let ring = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let first = ring.lines.partition_point(|line| line.seq <= seen);
        out.extend(ring.lines.range(first..).cloned());
        ring.last
    }

    /// The last line's number (0 before the first).
    pub fn last_seq(&self) -> u64 {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner).last
    }
}

/// The process's log, which [`install`]'s logger writes to.
pub fn global() -> &'static Arc<LogBuffer> {
    static GLOBAL: OnceLock<Arc<LogBuffer>> = OnceLock::new();
    GLOBAL.get_or_init(|| Arc::new(LogBuffer::new()))
}

/// Whether a record from `target` at `level` is kept for the Output panel: Metallic's own crates from `info`,
/// everything else from `warn`.
fn kept(target: &str, level: Level) -> bool {
    level <= Level::Warn || (target.starts_with("metallic") && level <= Level::Info)
}

/// Studio's logger: the [`global`] buffer, and stderr at `stderr`'s level.
struct StudioLogger {
    stderr: LevelFilter,
}

impl log::Log for StudioLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        kept(metadata.target(), metadata.level()) || self.to_stderr(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        let metadata = record.metadata();
        if self.to_stderr(metadata) {
            eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
        }
        if kept(metadata.target(), metadata.level()) {
            global().push(
                record.level(),
                record.target(),
                record.args().to_string(),
                Origin::Engine,
            );
        }
    }

    fn flush(&self) {}
}

impl StudioLogger {
    /// Whether stderr gets the record: Metallic's own crates at the level asked for, dependencies from `warn`.
    fn to_stderr(&self, metadata: &log::Metadata<'_>) -> bool {
        let ours = metadata.target().starts_with("metallic");
        metadata.level() <= self.stderr && (ours || metadata.level() <= Level::Warn)
    }
}

/// Makes Studio's logger the process's: the Output panel's buffer, and stderr at `stderr`'s level. Does nothing if a
/// logger is already set.
pub fn install(stderr: LevelFilter) {
    static LOGGER: OnceLock<StudioLogger> = OnceLock::new();
    let logger = LOGGER.get_or_init(|| StudioLogger { stderr });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(stderr.max(LevelFilter::Info));
    }
}

/// A game's script output, into a [`LogBuffer`]: what Studio hands the engine it runs a game in.
#[derive(Clone, Debug)]
pub struct ScriptOutput {
    buffer: Arc<LogBuffer>,
}

impl ScriptOutput {
    /// Script output into `buffer`.
    pub fn new(buffer: Arc<LogBuffer>) -> Self {
        Self { buffer }
    }

    fn push(&self, level: Level, from: &OutputSource<'_>, message: &str) {
        let source = match from.line {
            Some(line) => format!("{}:{line}", from.script),
            None => from.script.to_owned(),
        };
        self.buffer.push(level, source, message, Origin::Script);
    }
}

impl OutputSink for ScriptOutput {
    fn print(&mut self, from: &OutputSource<'_>, message: &str) {
        self.push(Level::Info, from, message);
    }

    fn warn(&mut self, from: &OutputSource<'_>, message: &str) {
        self.push(Level::Warn, from, message);
    }

    fn error(&mut self, from: &OutputSource<'_>, message: &str) {
        self.push(Level::Error, from, message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_numbered_and_read_once() {
        let buffer = LogBuffer::new();
        buffer.push(Level::Info, "metallic_project", "opened", Origin::Engine);
        buffer.push(Level::Warn, "metallic_project", "an asset is missing", Origin::Engine);
        let mut lines = Vec::new();
        let seen = buffer.read_since(0, &mut lines);
        assert_eq!(seen, 2);
        assert_eq!(lines[0].seq, 1);
        assert_eq!(lines[1].message, "an asset is missing");

        buffer.push(Level::Error, "metallic_render", "lost the device", Origin::Engine);
        let mut more = Vec::new();
        assert_eq!(buffer.read_since(seen, &mut more), 3);
        assert_eq!(more.len(), 1);
        assert_eq!(more[0].level, Level::Error);
        assert!(buffer.read_since(3, &mut more) == 3 && more.len() == 1, "nothing new");
    }

    #[test]
    fn the_ring_keeps_the_newest_lines() {
        let buffer = LogBuffer::new();
        for n in 0..LogBuffer::CAPACITY + 10 {
            buffer.push(Level::Info, "metallic", n.to_string(), Origin::Engine);
        }
        let mut lines = Vec::new();
        buffer.read_since(0, &mut lines);
        assert_eq!(lines.len(), LogBuffer::CAPACITY);
        assert_eq!(lines[0].message, "10");
        assert_eq!(buffer.last_seq(), (LogBuffer::CAPACITY + 10) as u64);
    }

    #[test]
    fn script_output_names_the_script_and_line() {
        let buffer = Arc::new(LogBuffer::new());
        let mut output = ScriptOutput::new(Arc::clone(&buffer));
        let from = OutputSource {
            script: "Workspace.Door.Script",
            chunk: "Scripts/Door.luau#1",
            line: Some(12),
        };
        output.print(&from, "hello");
        output.error(&OutputSource { line: None, ..from }, "boom");
        let mut lines = Vec::new();
        buffer.read_since(0, &mut lines);
        assert_eq!(lines[0].source, "Workspace.Door.Script:12");
        assert_eq!((lines[0].level, lines[0].origin), (Level::Info, Origin::Script));
        assert_eq!(lines[1].source, "Workspace.Door.Script");
        assert_eq!(lines[1].level, Level::Error);
    }

    #[test]
    fn only_metallics_own_info_is_kept() {
        assert!(kept("metallic_project::scene", Level::Info));
        assert!(!kept("metallic_project::scene", Level::Debug));
        assert!(!kept("wgpu_core::device", Level::Info));
        assert!(kept("wgpu_core::device", Level::Warn));
    }
}
