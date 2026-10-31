//! The Output panel and its tagged, non-blocking sink.
//!
//! # Seam: feeding Output from anywhere
//!
//! Every line carries a [`Tag`] (where it came from: `Studio`, `Server`, `Client 1`, ...) and a
//! [`Level`]. Producers hold an [`OutputSink`], a cheap clonable handle over a bounded channel:
//!
//! ```
//! use metallic_studio::output::{Level, OutputLog, Tag};
//! let mut log = OutputLog::new(1024);
//! let sink = log.sink();
//! std::thread::spawn(move || {
//!     sink.push(Tag::Client(2), Level::Info, "joined as Player2");
//! })
//! .join()
//! .unwrap();
//! log.drain();
//! assert_eq!(log.lines().count(), 1);
//! ```
//!
//! [`OutputSink::push`] never blocks: when the channel is full the line is dropped and counted
//! ([`OutputLog::dropped`]), and the panel shows "N lines dropped". A process reader thread (Team
//! Test, task A12) pushes each child's stdout with its own tag; [`ScriptOutput`] adapts a sink to
//! the Luau VM's `print`/`warn`/`error` for Play (#11). The UI thread calls [`OutputLog::drain`]
//! once per frame. Repeats of the same line from the same source collapse into one row with a
//! count. The panel filters by tag (colour chips, one per tag seen) and by level, and by text.
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::Instant;

use eframe::egui::{self, Color32, RichText};

use crate::tokens::ui_tokens;

/// Default channel capacity (lines in flight between producers and the UI).
pub const CHANNEL_CAPACITY: usize = 4096;
/// Default number of rows kept in the panel.
pub const KEEP_LINES: usize = 10_000;

/// Severity of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    /// `print` and Studio notes.
    Info,
    /// `warn`.
    Warning,
    /// Errors.
    Error,
}

/// Where a line came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tag {
    /// Studio itself (save, load, tool messages).
    Studio,
    /// The game server (Play's server side, Team Test's server process).
    Server,
    /// A client by number, 1-based ("Client 1").
    Client(u8),
    /// Anything else, named.
    Other(Arc<str>),
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tag::Studio => f.write_str("Studio"),
            Tag::Server => f.write_str("Server"),
            Tag::Client(n) => write!(f, "Client {n}"),
            Tag::Other(s) => f.write_str(s),
        }
    }
}

impl Tag {
    /// Chip colour: muted, distinct hues so tags tell apart at a glance (the text names them
    /// too, so colour is never the only signal).
    pub fn colour(&self) -> Color32 {
        let t = ui_tokens();
        match self {
            Tag::Studio => t.text_dim,
            Tag::Server => t.accent,
            Tag::Client(n) => t.clients[usize::from(n.saturating_sub(1)) % t.clients.len()],
            Tag::Other(_) => t.other_tag,
        }
    }
}

/// One Output row.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Seconds since the log was created.
    pub time: f64,
    /// Source.
    pub tag: Tag,
    /// Severity.
    pub level: Level,
    /// Message.
    pub text: String,
    /// How many times this exact line arrived in a row (1 = once).
    pub repeats: u32,
}

/// A clonable, thread-safe handle that sends lines to Output without blocking.
#[derive(Clone)]
pub struct OutputSink {
    tx: SyncSender<Line>,
    dropped: Arc<AtomicU64>,
    start: Instant,
}

impl OutputSink {
    /// Queues a line. Returns false (and counts it) when the queue is full or Output is gone.
    pub fn push(&self, tag: Tag, level: Level, text: impl Into<String>) -> bool {
        let line = Line {
            time: self.start.elapsed().as_secs_f64(),
            tag,
            level,
            text: text.into(),
            repeats: 1,
        };
        match self.tx.try_send(line) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Shorthand for a Studio info line.
    pub fn studio(&self, text: impl Into<String>) {
        self.push(Tag::Studio, Level::Info, text);
    }
}

/// Adapts an [`OutputSink`] to the Luau VM's output (`print`, `warn`, script errors) with a fixed
/// tag, for Play sessions.
pub struct ScriptOutput {
    /// Where lines go.
    pub sink: OutputSink,
    /// The tag every line gets.
    pub tag: Tag,
}

impl metallic_script::OutputSink for ScriptOutput {
    fn print(&self, line: &str) {
        self.sink.push(self.tag.clone(), Level::Info, line);
    }
    fn warn(&self, line: &str) {
        self.sink.push(self.tag.clone(), Level::Warning, line);
    }
    fn error(&self, line: &str) {
        self.sink.push(self.tag.clone(), Level::Error, line);
    }
}

/// The receiving end: keeps the newest lines for the panel.
pub struct OutputLog {
    rx: Receiver<Line>,
    sink: OutputSink,
    lines: VecDeque<Line>,
    keep: usize,
    /// Tags seen so far, in first-seen order (the filter chips).
    tags: Vec<Tag>,
    counts: [usize; 3],
    revision: u64,
}

impl OutputLog {
    /// A log keeping up to `keep` rows, with a channel of [`CHANNEL_CAPACITY`] lines.
    pub fn new(keep: usize) -> OutputLog {
        OutputLog::with_capacity(keep, CHANNEL_CAPACITY)
    }

    /// A log with an explicit channel capacity.
    pub fn with_capacity(keep: usize, capacity: usize) -> OutputLog {
        let (tx, rx) = sync_channel(capacity.max(1));
        OutputLog {
            rx,
            sink: OutputSink {
                tx,
                dropped: Arc::new(AtomicU64::new(0)),
                start: Instant::now(),
            },
            lines: VecDeque::new(),
            keep: keep.max(1),
            tags: vec![Tag::Studio],
            counts: [0; 3],
            revision: 0,
        }
    }

    /// A new handle for a producer.
    pub fn sink(&self) -> OutputSink {
        self.sink.clone()
    }

    /// Moves queued lines into the log. Returns true when anything arrived.
    pub fn drain(&mut self) -> bool {
        let mut any = false;
        while let Ok(line) = self.rx.try_recv() {
            any = true;
            if !self.tags.contains(&line.tag) {
                self.tags.push(line.tag.clone());
            }
            if let Some(last) = self.lines.back_mut()
                && last.tag == line.tag
                && last.level == line.level
                && last.text == line.text
            {
                last.repeats += 1;
                last.time = line.time;
                continue;
            }
            self.counts[line.level as usize] += 1;
            self.lines.push_back(line);
            if self.lines.len() > self.keep
                && let Some(old) = self.lines.pop_front()
            {
                self.counts[old.level as usize] -= 1;
            }
        }
        if any {
            self.revision += 1;
        }
        any
    }

    /// Rows, oldest first.
    pub fn lines(&self) -> impl DoubleEndedIterator<Item = &Line> + ExactSizeIterator {
        self.lines.iter()
    }

    /// Tags seen so far.
    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }

    /// Lines lost because the channel was full.
    pub fn dropped(&self) -> u64 {
        self.sink.dropped.load(Ordering::Relaxed)
    }

    /// Removes every row (Clear).
    pub fn clear(&mut self) {
        self.lines.clear();
        self.counts = [0; 3];
        self.revision += 1;
    }

    /// Number of rows at `level`.
    pub fn count(&self, level: Level) -> usize {
        self.counts[level as usize]
    }
}

/// Panel state: the log plus filters.
pub struct OutputPanel {
    /// The lines.
    pub log: OutputLog,
    hidden_tags: Vec<Tag>,
    show: [bool; 3],
    filter: String,
    visible: Vec<usize>,
    visible_for: (u64, u64),
    filter_revision: u64,
}

impl OutputPanel {
    /// A panel with an empty log.
    pub fn new() -> OutputPanel {
        OutputPanel {
            log: OutputLog::new(KEEP_LINES),
            hidden_tags: Vec::new(),
            show: [true; 3],
            filter: String::new(),
            visible: Vec::new(),
            visible_for: (u64::MAX, u64::MAX),
            filter_revision: 0,
        }
    }

    fn passes(&self, line: &Line) -> bool {
        self.show[line.level as usize]
            && !self.hidden_tags.contains(&line.tag)
            && (self.filter.is_empty() || contains_ignore_case(&line.text, &self.filter))
    }

    fn refresh_visible(&mut self) {
        let key = (self.log.revision, self.filter_revision);
        if self.visible_for == key {
            return;
        }
        self.visible.clear();
        for (i, line) in self.log.lines.iter().enumerate() {
            if self.passes(line) {
                self.visible.push(i);
            }
        }
        self.visible_for = key;
    }

    /// Draws the panel.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.log.drain();
        ui.spacing_mut().interact_size.y = crate::theme::ROW_HEIGHT + 4.0;
        ui.horizontal(|ui| {
            let mut changed = false;
            for (i, (label, level)) in [
                ("Messages", Level::Info),
                ("Warnings", Level::Warning),
                ("Errors", Level::Error),
            ]
            .into_iter()
            .enumerate()
            {
                let text = format!("{label} {}", self.log.count(level));
                changed |= crate::panel::chip(ui, &text, &mut self.show[i], None);
            }
            ui.separator();
            let tags: Vec<Tag> = self.log.tags().to_vec();
            for tag in tags {
                let mut on = !self.hidden_tags.contains(&tag);
                if crate::panel::chip(ui, &tag.to_string(), &mut on, Some(tag.colour())) {
                    changed = true;
                    if on {
                        self.hidden_tags.retain(|t| t != &tag);
                    } else {
                        self.hidden_tags.push(tag);
                    }
                }
            }
            ui.separator();
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("Filter output")
                        .desired_width(160.0),
                )
                .changed();
            if ui
                .button("Clear")
                .on_hover_text("Clear Output (Ctrl+L)")
                .clicked()
            {
                self.log.clear();
            }
            if self.log.dropped() > 0 {
                ui.label(
                    RichText::new(format!("{} lines dropped", self.log.dropped()))
                        .color(ui_tokens().text_dim),
                );
            }
            if changed {
                self.filter_revision += 1;
            }
        });
        self.refresh_visible();
        let row_h = ui_tokens().row_height;
        let lines = &self.log.lines;
        let visible = &self.visible;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_rows(ui, row_h, visible.len(), |ui, range| {
                for &i in &visible[range] {
                    let line = &lines[i];
                    let bg = if line.level == Level::Error {
                        ui_tokens().error_bg
                    } else {
                        Color32::TRANSPARENT
                    };
                    egui::Frame::new().fill(bg).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format_time(line.time))
                                    .monospace()
                                    .color(ui_tokens().text_dim),
                            );
                            ui.label(RichText::new(line.tag.to_string()).color(line.tag.colour()));
                            let colour = match line.level {
                                Level::Info => ui_tokens().text,
                                Level::Warning => ui_tokens().warning,
                                Level::Error => ui_tokens().error,
                            };
                            ui.label(RichText::new(&line.text).monospace().color(colour));
                            if line.repeats > 1 {
                                ui.label(
                                    RichText::new(format!("×{}", line.repeats))
                                        .small()
                                        .color(ui_tokens().text_dim),
                                );
                            }
                        });
                    });
                }
            });
    }
}

impl Default for OutputPanel {
    fn default() -> Self {
        OutputPanel::new()
    }
}

fn format_time(t: f64) -> String {
    let total = t as u64;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total / 60) % 60,
        total % 60
    )
}

fn contains_ignore_case(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_channel_drops_and_counts_without_blocking() {
        let mut log = OutputLog::with_capacity(100, 2);
        let sink = log.sink();
        assert!(sink.push(Tag::Server, Level::Info, "a"));
        assert!(sink.push(Tag::Server, Level::Info, "b"));
        assert!(!sink.push(Tag::Server, Level::Info, "c"));
        assert_eq!(log.dropped(), 1);
        log.drain();
        assert_eq!(log.lines().count(), 2);
        assert_eq!(log.tags(), &[Tag::Studio, Tag::Server]);
    }

    #[test]
    fn repeats_collapse_and_old_rows_fall_off() {
        let mut log = OutputLog::new(2);
        let sink = log.sink();
        for _ in 0..3 {
            sink.push(Tag::Client(1), Level::Error, "boom");
        }
        sink.push(Tag::Client(2), Level::Info, "x");
        sink.push(Tag::Client(2), Level::Info, "y");
        log.drain();
        let rows: Vec<&Line> = log.lines().collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text, "x");
        log.clear();
        sink.push(Tag::Client(1), Level::Error, "boom");
        sink.push(Tag::Client(1), Level::Error, "boom");
        log.drain();
        assert_eq!(log.lines().next().unwrap().repeats, 2);
    }

    #[test]
    fn tags_display_names() {
        assert_eq!(Tag::Client(3).to_string(), "Client 3");
        assert_eq!(Tag::Other("Bot".into()).to_string(), "Bot");
    }
}
